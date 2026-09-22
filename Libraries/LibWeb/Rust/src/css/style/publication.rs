/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

mod drive;
pub(super) mod pending;
mod pseudo;
mod winner_store;

use winner_store::{WinnerDeclaration, WinnerStore, WinnerValue, shorthand_longhand_data};

use super::*;
use crate::css::computed_longhand_table::ComputedLonghandTable;
use drive::FontDriveGoal;
pub(crate) use drive::drive_font_metric;

/// Another element's published style that a first-time computation may build over: the element
/// whose cascade state stands in for the previous one, and the record it must still hold.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExactCascadeDonor {
    pub node: StyleNodeID,
    pub style_record: u64,
}

pub(super) struct ExactCascadeContext {
    previous: Option<CascadeStateID>,
    lower_bound_state: Option<CascadeStateID>,
    dependency_target: computed::ComputedStyleTarget,
    donor_used: bool,
}

/// What a record the engine settled leaves in place of the style input record a C++ computation
/// would have written.
///
/// An engine-settled record never runs the C++ computation, so the element's next one has nothing
/// to compare against and rebuilds every group. This is the comparison it would have made: the
/// parent's record, the element's own adjustment facts and the document environment as they were
/// when the record was settled. When all of them still hold, nothing but the element's own
/// declarations can have moved, which is exactly what `take_shared_computation_context` answers for
/// a record another element published.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct SettledComputationContext {
    record: u64,
    parent_record: u64,
    parent_display: Option<u32>,
    adjustment_facts: u32,
    environment: u64,
    font_environment_generation: u64,
    root_font_inputs: RootFontInputs,
    tree_scope: u32,
}

/// Root-relative computation reads these inputs independently of its inheritance parent.
/// Bitwise keys keep equality exact without using an invalidation generation as a substitute
/// for the values. The viewport-dependence bit matters even when today's metrics agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct RootFontInputs {
    metrics: [u64; 5],
    depends_on_viewport: bool,
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

    pub(crate) fn retained_inheritance_parent_style_record(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Option<computed::FinalStyleRecordID> {
        let parent = self.retained_inheritance_parent_node(node, pseudo_kind)?;
        if let Some(record) = self
            .legacy_finalized_longhand_rows
            .get(&computed::ComputedStyleTarget::new(parent, u8::MAX))
            .and_then(|row| computed::FinalStyleRecordID::from_raw(row.assembled_style_record))
        {
            return Some(record);
        }
        self.computed_group_sets.assigned_style_record(parent)
    }

    pub(crate) fn retained_legacy_inheritance_parent_table(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Option<(
        &crate::css::computed_longhand_table::ComputedLonghandTable,
        u64,
        u64,
        u64,
    )> {
        let parent = self.retained_inheritance_parent_node(node, pseudo_kind)?;
        self.legacy_finalized_longhand_rows
            .get(&computed::ComputedStyleTarget::new(parent, u8::MAX))
            .map(|row| {
                (
                    row.table(),
                    row.previous_style_record,
                    row.assembled_style_record,
                    self.computed_group_sets
                        .assigned_style_record(parent)
                        .map_or(0, computed::FinalStyleRecordID::raw),
                )
            })
    }

    pub(crate) unsafe fn retain_legacy_finalized_longhand_row(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        table: *const crate::css::computed_longhand_table::ComputedLonghandTable,
        previous_style_record: u64,
        assembled_style_record: u64,
    ) {
        self.legacy_finalized_longhand_rows
            .insert(computed::ComputedStyleTarget::new(node, pseudo_kind), unsafe {
                LegacyFinalizedLonghandRow::retain(table, previous_style_record, assembled_style_record)
            });
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

    /// The retained style records, root first, whose raw cascaded font sizes participate in the
    /// monospace font-size recascade for one element or pseudo-element.
    pub(crate) fn retained_inheritance_ancestor_style_records(&self, node: StyleNodeID, pseudo_kind: u8) -> Vec<u64> {
        let mut records = Vec::new();
        let mut ancestor = self.retained_inheritance_parent_node(node, pseudo_kind);
        while let Some(node) = ancestor {
            records.push(
                self.computed_group_sets
                    .assigned_style_record(node)
                    .map_or(0, computed::FinalStyleRecordID::raw),
            );
            ancestor = self.tree.inheritance_parent(node);
        }
        records.reverse();
        records
    }

    fn shared_style_record_key(
        &self,
        node: StyleNodeID,
        parent_record: u64,
        environment: u64,
        shape: [u64; 4],
    ) -> Option<computed::SharedStyleRecordKey> {
        if !self.has_no_element_declarations(node) || self.node_declares_custom_properties(node) {
            return None;
        }
        let cascade_input = self.current_published_answer(node)?.cascade_input?;
        Some(computed::SharedStyleRecordKey {
            cascade_input: cascade_input.0,
            tree_scope: self.tree.tree_scope(node).0,
            inherited_groups: self
                .computed_group_sets
                .inherited_groups_for_shared_style(parent_record)?,
            environment,
            font_environment_generation: self.document_style_computation_inputs?.font_environment_generation,
            root_font_inputs: RootFontInputs::from_document(&self.document_style_computation_inputs?),
            shape,
        })
    }

    pub(crate) fn lookup_shared_style_record(
        &mut self,
        node: StyleNodeID,
        parent_record: u64,
        environment: u64,
        shape: [u64; 4],
        counters: &mut Counters,
    ) -> Option<u64> {
        let key = self.shared_style_record_key(node, parent_record, environment, shape)?;
        let shared = self.computed_group_sets.shared_style_record(key)?;
        let inherited_group_count = self
            .computed_group_sets
            .inherited_group_count_for_shared_style(shared.record)?;
        self.mark_published_answer_observed(node);
        self.prepare_shared_exact_cascade_state(node);
        self.forget_engine_computed_record(computed::ComputedStyleTarget::new(node, u8::MAX));
        let publication = self.assign_shared_style_record(
            computed::ComputedStyleTarget::new(node, u8::MAX),
            shared.record,
            inherited_group_count,
            shared.inherited_group_swap_eligible,
            counters,
        );
        let record = publication.style_record_identity.raw();
        self.computed_group_sets.remember_shared_computation_context(
            node,
            computed::SharedComputationContext {
                parent_record,
                record,
                key,
            },
        );
        self.settle_computed_memory();
        counters.bump(Counter::SharedStyleRecordHits);
        Some(record)
    }

    pub(crate) fn take_shared_computation_context(
        &mut self,
        node: StyleNodeID,
        parent_record: u64,
        environment: u64,
        shape: [u64; 4],
        pseudo_elements: u64,
    ) -> Option<u64> {
        if let Some(record) =
            self.take_published_shared_computation_context(node, parent_record, environment, shape, pseudo_elements)
        {
            return Some(record);
        }
        self.take_settled_computation_context(node, parent_record, environment, pseudo_elements)
    }

    /// The context a record the engine settled left, for the element's next C++ computation: the
    /// record itself when everything that computation would have compared still holds, so that the
    /// only thing that can have moved is the element's own declarations.
    fn take_settled_computation_context(
        &mut self,
        node: StyleNodeID,
        parent_record: u64,
        environment: u64,
        pseudo_elements: u64,
    ) -> Option<u64> {
        let context = self.settled_computation_contexts.remove(&node)?;
        let inputs = self.document_style_computation_inputs?;
        if context.environment != environment
            || context.font_environment_generation != inputs.font_environment_generation
            || context.root_font_inputs != RootFontInputs::from_document(&inputs)
            || context.tree_scope != self.tree.tree_scope(node).0
            || context.parent_record != parent_record
            || context.adjustment_facts != self.computed_group_sets.adjustment_facts(node)
        {
            return None;
        }
        // The box type a value was transformed under is the parent's display, which the record does
        // not name and the cascade cannot report.
        let parent_display = self
            .tree
            .flat_tree_parent(node)
            .and_then(|parent| self.box_type_parent_display(parent));
        if context.parent_display != parent_display
            || self.computed_group_sets.assigned_style_record(node)?.raw() != context.record
            || self
                .computed_group_sets
                .style_record_view(context.record)?
                .pseudo_element_styles
                != pseudo_elements
        {
            return None;
        }
        Some(context.record)
    }

    /// Remember, beside a record the engine settled, what the element's next C++ computation has to
    /// compare to know that nothing but its declarations moved.
    ///
    /// Only for a record the engine can vouch for the way the host vouches for a shared one: no
    /// custom-property substitution and no explicit `inherit` of a non-inherited property, which are
    /// the two things that reach past what the record names. Everything else a computation could
    /// read past the record - a container unit, a tree-counting function, a resource context - is
    /// already refused by the gates that let the engine settle the record at all.
    fn remember_settled_computation_context(&mut self, node: StyleNodeID) {
        if self.nodes_with_substituted_records.contains(&node)
            || self.node_explicitly_inherits_non_inherited_property(node)
            || self.node_declares_custom_properties(node)
        {
            self.settled_computation_contexts.remove(&node);
            return;
        }
        let Some(inputs) = self.document_style_computation_inputs else {
            return;
        };
        let Some(record) = self.computed_group_sets.assigned_style_record(node) else {
            return;
        };
        let parent = self.tree.flat_tree_parent(node);
        let parent_record = parent
            .and_then(|parent| self.computed_group_sets.assigned_style_record(parent))
            .map_or(0, |record| record.raw());
        let parent_display = parent.and_then(|parent| self.box_type_parent_display(parent));
        let context = SettledComputationContext {
            record: record.raw(),
            parent_record,
            parent_display,
            adjustment_facts: self.computed_group_sets.adjustment_facts(node),
            environment: inputs.style_environment_version,
            font_environment_generation: inputs.font_environment_generation,
            root_font_inputs: RootFontInputs::from_document(&inputs),
            tree_scope: self.tree.tree_scope(node).0,
        };
        self.settled_computation_contexts.insert(node, context);
    }

    fn take_published_shared_computation_context(
        &mut self,
        node: StyleNodeID,
        parent_record: u64,
        environment: u64,
        shape: [u64; 4],
        pseudo_elements: u64,
    ) -> Option<u64> {
        let context = self.computed_group_sets.take_shared_computation_context(node)?;
        if context.key.environment != environment
            || context.key.font_environment_generation
                != self.document_style_computation_inputs?.font_environment_generation
            || context.key.root_font_inputs != RootFontInputs::from_document(&self.document_style_computation_inputs?)
            || context.key.tree_scope != self.tree.tree_scope(node).0
            || context.key.shape[..3] != shape[..3]
            || self.computed_group_sets.assigned_style_record(node)?.raw() != context.record
        {
            return None;
        }
        let previous_inherited = self
            .computed_group_sets
            .inherited_groups_for_shared_style(context.parent_record)?;
        let current_inherited = self
            .computed_group_sets
            .inherited_groups_for_shared_style(parent_record)?;
        if previous_inherited != current_inherited
            || self
                .computed_group_sets
                .style_record_view(context.record)?
                .pseudo_element_styles
                != pseudo_elements
        {
            return None;
        }
        Some(context.record)
    }

    pub(crate) fn remember_shared_style_record(
        &mut self,
        node: StyleNodeID,
        parent_record: u64,
        environment: u64,
        shape: [u64; 4],
        record: u64,
    ) {
        let Some(key) = self.shared_style_record_key(node, parent_record, environment, shape) else {
            return;
        };
        self.computed_group_sets.remember_shared_style_record(node, key, record);
        self.settle_computed_memory();
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
    ) -> Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)> {
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
    ) -> Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)> {
        if scratch.root_computation_unsupported == Some(node) {
            counters.bump(Counter::EngineComputedRecordBailRootFontInputs);
            return None;
        }
        let pending_element = scratch.pending_element.take();
        if pending_element.is_none() {
            scratch.pseudo_deltas.clear();
            scratch.next_pseudo = 0;
            scratch.pseudo_uses_substitution = false;
            scratch.noted_substitution = None;
            scratch.flipped_pseudo_rules = exact_flipped_rules.map_or(0, |flipped| flipped.pseudos);
            if parent_inputs_moved.inherited_style && !self.engine_marker_font_supported(node, counters) {
                return None;
            }
            if !self.engine_pseudo_inputs_available(
                node,
                self.computed_group_sets.assigned_style_record(node),
                counters,
            ) {
                return None;
            }
        }
        let delta = match pending_element {
            Some(delta) => delta,
            None => self.engine_computed_element_record_delta(
                node,
                cascade_winners_are_complete,
                exact_flipped_rules,
                parent_inputs_moved,
                scratch,
                FontDriveGoal::Complete,
                counters,
            )?,
        };
        // The element's pseudo-elements are settled beside its record, as the C++ computation
        // refreshes them after the element's own; a pseudo-element the engine cannot settle
        // sends the whole element to C++.
        let old_style_record = (delta.0 != computed::FinalStyleRecordID::NONE).then_some(delta.0);
        let generation = self.winner_groups.generation();
        if self
            .engine_pseudo_records(node, old_style_record, delta.1, generation, scratch, counters)
            .is_none()
        {
            if scratch.font_drive.request.is_some() {
                scratch.pending_element = Some(delta);
            } else {
                self.abandon_engine_computed_record(node, scratch, counters);
            }
            return None;
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
        Some(delta)
    }

    /// `exact_flipped_rules` are the rules that flipped for the node when the reaction is exactly
    /// those flips and nothing else the record depends on moved. `parent_inputs_moved` says which
    /// of the parent's inputs may have moved under the record.
    #[allow(clippy::too_many_arguments)]
    fn engine_computed_element_record_delta(
        &mut self,
        node: StyleNodeID,
        cascade_winners_are_complete: bool,
        exact_flipped_rules: Option<FlippedRules>,
        mut parent_inputs_moved: ParentInputsMoved,
        scratch: &mut EngineComputedRecordScratch,
        goal: FontDriveGoal,
        counters: &mut Counters,
    ) -> Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)> {
        use crate::css::computed_value_types::{
            STYLE_GROUP_INDEX_ANCHOR, STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_SURROUND,
        };
        use crate::css::computed_values::computed_group_dependency_mask;
        use crate::css::property_metadata::{FIRST_LONGHAND_PROPERTY_ID, LONGHAND_WORD_COUNT};

        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        // A custom property the cascade declares is no winner the columns hold; the engine
        // computes the environment it decides itself.
        if !cascade_winners_are_complete && !self.cascade_winners_are_complete_but_for_custom_properties(node) {
            counters.bump(Counter::EngineComputedRecordBailIncompleteWinners);
            return None;
        }
        // The winners the record was computed from, against the winners the node holds now: the
        // same comparison a C++ publication makes to select what it recomputes.
        let (generation, state) = match self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        {
            Lookup::Known(token) => token,
            Lookup::Missing(gap) => {
                counters.bump(match gap {
                    cascade::WinnerGroupGap::MissingNode(_) => Counter::EngineComputedRecordBailWinnerMissingNode,
                    cascade::WinnerGroupGap::StaleProgram { .. } => Counter::EngineComputedRecordBailWinnerStaleProgram,
                    cascade::WinnerGroupGap::StalePriority(_) => Counter::EngineComputedRecordBailWinnerStalePriority,
                });
                return None;
            }
            Lookup::KnownAbsent => {
                counters.bump(Counter::EngineComputedRecordBailWinner);
                return None;
            }
        };
        // An element's animations compose into its style in the C++ computation, an element
        // standing for its host's pseudo-element takes the style C++ computes for that
        // pseudo-element, and a hint mapped from another element's attributes moves without
        // anything recorded on the element.
        let facts = self.computed_group_sets.adjustment_facts(node);
        // Either the root's font inputs moved under this element, or the element's own font
        // environment did: a face its cascade names became available or failed. The winners are
        // the same either way, so nothing else below would notice, and the record has to be
        // driven again in full rather than stand.
        let font_inputs_moved = (scratch.root_font_inputs_changed
            && facts & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT == 0)
            || scratch.font_environment_moved;
        if facts
            & (bridge::element_adjustment_fact::IS_SHADOW_HOST_PSEUDO_ELEMENT
                | bridge::element_adjustment_fact::HAS_DERIVED_PRESENTATIONAL_HINTS)
            != 0
        {
            counters.bump(Counter::EngineComputedRecordBailWinnerElement);
            return None;
        }
        // An element's animations compose into its style in the C++ computation, and the record it
        // holds is the one they were composed into. Deriving another record from it, or moving it
        // to another environment, would publish the composition as if it were the element's own
        // style - so all of that stays in C++ while the element animates. Answering with the very
        // record the element already holds does not: it is what the row asks for when nothing the
        // record was computed from has moved, and the overlay on it is the host's either way.
        let animations_bind_the_record = facts & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0;
        let Some(old_style_record) = self.computed_group_sets.assigned_style_record(node) else {
            // Presentational hints are mapped from the attributes by the C++ computation, which
            // publishes them as the element's declarations: a first record waits for that
            // computation, and an attribute change asks for it through a recorded input, so a
            // later record's winners carry the hints.
            // A first record for an element that already animates would be a record the engine
            // computed for a style the host composes into, and the plan it may leave was decided
            // against the animations the host published, which are not the only ones it holds.
            if facts & bridge::element_adjustment_fact::HAS_PRESENTATIONAL_HINTS != 0 || animations_bind_the_record {
                counters.bump(Counter::EngineComputedRecordBailWinnerElement);
                return None;
            }
            return self.engine_cold_record(node, (generation, state), scratch, goal, counters);
        };
        // The same of a record that holds an animation overlay, or whose table declares a
        // transition the moved values would start.
        let animations_bind_the_record =
            animations_bind_the_record || self.record_requires_cpp_animation(old_style_record);
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
        // A record derived from the old one inherits what the parent's animations sampled when the
        // old one was computed.
        if self.parent_composes_animations(node) {
            counters.bump(Counter::EngineComputedRecordBailRecordOverlay);
            return None;
        }
        // A record C++ computed holds no cascade state; when the reaction moved none of the
        // node's own rules its winners are the ones the record was computed from, and a full
        // drive against the moved parent inputs binds the state.
        let winners_unchanged = exact_flipped_rules.is_some_and(|flipped| !flipped.element);
        let delta = match self.computed_group_sets.cascade_state(target) {
            Some((previous_generation, previous_state)) => {
                if previous_generation != generation {
                    counters.bump(Counter::EngineComputedRecordBailStaleCascadeState);
                    return None;
                }
                self.winner_groups.semantic_delta(Some(previous_state), state)
            }
            None if winners_unchanged
                && (parent_inputs_moved.any()
                    || font_inputs_moved
                    || scratch.document_environment_moved
                    || scratch.recompute_in_full) =>
            {
                self.winner_groups.semantic_delta(Some(state), state)
            }
            None => {
                counters.bump(Counter::EngineComputedRecordBailNoCascadeState);
                return None;
            }
        };
        let Some(mut inputs) = self.document_style_computation_inputs else {
            counters.bump(Counter::EngineComputedRecordBailNoEnvironment);
            return None;
        };
        if let Some((root, root_inputs)) = scratch.root_element_inputs
            && root == node
        {
            root_inputs.apply_to(&mut inputs);
        }
        // The environment the node's own custom declarations resolve to over the parent's. A node
        // declaring none keeps its record's, which is the parent's; a moved environment
        // republishes the record under the new one.
        let environment = {
            let parent_environment = match self.tree.flat_tree_parent(node) {
                Some(parent) => {
                    let Some(parent_environment) =
                        self.computed_group_sets.custom_property_environment_identity(parent)
                    else {
                        counters.bump(Counter::EngineComputedRecordBailRecordParent);
                        return None;
                    };
                    parent_environment
                }
                None => 0,
            };
            let Some(environment) =
                self.engine_custom_property_environment(node, parent_environment, &inputs, counters)
            else {
                counters.bump(Counter::EngineComputedRecordBailCustomProperties);
                return None;
            };
            // A record an animation composed into was published with no environment of its own:
            // what the element's own declarations resolved to is on the style beneath it.
            let Some(old_environment) = self
                .computed_group_sets
                .animation_overlay_base_custom_property_environment(old_style_record.raw())
                .or_else(|| {
                    self.computed_group_sets
                        .style_record_custom_property_environment(old_style_record.raw())
                })
            else {
                counters.bump(Counter::EngineComputedRecordBailRecord);
                return None;
            };
            (environment != old_environment).then_some(environment)
        };
        let Some(current_environment) = environment.or_else(|| {
            self.computed_group_sets
                .animation_overlay_base_custom_property_environment(old_style_record.raw())
                .or_else(|| {
                    self.computed_group_sets
                        .style_record_custom_property_environment(old_style_record.raw())
                })
        }) else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return None;
        };
        // A moved environment reaches every winner written with a substitution: such a record
        // is driven again in full under the new one.
        let environment_moved_under_substitutions = environment.is_some() && self.state_has_substitutions(node, state);
        if delta.is_empty() {
            // The winners the record was computed from are the winners now. When everything else
            // the record was computed from is as it was too - the document environment, the rules
            // that flipped (custom properties are no winners), the parent's inherited style and
            // custom-property environment - the record stands, and the reaction may still move a
            // pseudo-element. The state has to hold the flips: a row this flush published holds
            // the cascade of the node's current answer, whichever rules flipped for it. Anything
            // else recomputes in C++, as does a record under a moved environment, which reaches
            // values its winners do not name.
            let row_is_current = self.current_winner_groups().row_stamp(node) == Some(self.flush_stamp);
            let flips_are_reflected = match exact_flipped_rules {
                Some(flipped) => !flipped.element || row_is_current,
                None => row_is_current && !scratch.environment_changed,
            };
            if !flips_are_reflected {
                counters.bump(Counter::EngineComputedRecordBailUnchangedWinners);
                // The document element's own record stays with C++, but nothing here says its
                // font inputs moved: keep the host's root-metric route rather than declaring the
                // root's computation unsupported, which takes every descendant with it.
                if goal == FontDriveGoal::RootInputs {
                    scratch.font_drive.root_inputs_unproven = true;
                }
                return None;
            }
            // The winners stand while the parent's inherited style or display moved under the
            // record: it is driven again in full against the parent as it is now. The record
            // does not say which parent display it was transformed under, and a winner's own
            // value may read the parent (a relative length, an inherit keyword).
            if !parent_inputs_moved.any()
                && !font_inputs_moved
                && !environment_moved_under_substitutions
                && !scratch.document_environment_moved
                && !scratch.recompute_in_full
            {
                // A declaration in an inherited payload group does not prove that the other
                // properties in that group still inherit from the current parent. Re-drive the
                // record in full when its payloads cannot prove the relationship.
                if self.record_inherits_from_current_parent(node, state, 0) {
                    if goal == FontDriveGoal::RootInputs {
                        // NB: This proof covers the retained font, without publishing the root's
                        //     remaining properties or custom-property environment during preparation.
                        scratch.font_drive.root_inputs = self.root_font_inputs_from_record(old_style_record);
                        return None;
                    }
                    // Only the environment moved: the record keeps its groups and takes the new one.
                    // A record the element's animations compose into is not the engine's to move.
                    if environment.is_some() && animations_bind_the_record {
                        counters.bump(Counter::EngineComputedRecordBailRecordOverlay);
                        return None;
                    }
                    if let Some(environment) = environment {
                        let Some(delta) = self
                            .computed_group_sets
                            .republish_engine_record_with_environment(node, environment)
                        else {
                            counters.bump(Counter::EngineComputedRecordBailAssemble);
                            return None;
                        };
                        counters.bump(Counter::EngineComputedRecordUnchangedWinners);
                        self.note_engine_computed_record(node, delta, (generation, state), 0, 0, counters);
                        return Some(delta);
                    }
                    // The record the row answers with has to be one the host can still read when
                    // the batch installs it. An animation overlay's record lives in a slot the next
                    // sampling of that animation releases, and a sampling runs between the flush
                    // that settles this row and the batch that applies it: answering with one hands
                    // the element a record that has stopped existing. The style beneath it is not an
                    // answer either - installing it would drop the animation for a frame.
                    if !record_may_stand_while_animating
                        || computed::ComputedGroupSets::record_is_animation_overlay(old_style_record.raw())
                    {
                        counters.bump(Counter::EngineComputedRecordBailRecordOverlay);
                        return None;
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
                    return Some((old_style_record, old_style_record));
                }
                // Otherwise the parent's inherited style moved under the record without the row
                // being told: the parent is authoritative here (an ancestor the host drives in this
                // batch holds the row back before it gets this far), so the record is driven again
                // in full against it.
                parent_inputs_moved.inherited_style = true;
            }
        }
        // Past the record that stands, every route derives another one from the record the element
        // holds. The values its animations composed are in that record, so deriving one is only
        // honest where the delta moves nothing those animations write: the base beneath them moves,
        // the composition over it does not, and the host samples it again over the new base once
        // the batch is applied.
        let overlay_animates_a_moved_property = self
            .computed_group_sets
            .style_record_view(old_style_record.raw())
            .and_then(|view| unsafe { view.animated_overlay.as_ref() })
            .is_some_and(|overlay| {
                overlay
                    .entries()
                    .iter()
                    .any(|entry| delta.properties().contains(&entry.property))
            });
        let derived_beneath_a_composition = animations_bind_the_record
            && !overlay_animates_a_moved_property
            && self.computed_group_sets.node_has_animation_overlay(node)
            && !self.css_defined_animations.node_runs_a_css_animation(node);
        let animations_bind_the_record = animations_bind_the_record && !derived_beneath_a_composition;
        if animations_bind_the_record {
            counters.bump(Counter::EngineComputedRecordBailWinnerElement);
            return None;
        }
        // A moved font-phase longhand reaches every value the font feeds, so the record is driven
        // through every phase and every group is rebuilt. A moved box-type transformation input
        // takes the same route, as does a record whose parent inputs moved: the transformation
        // and the inheritance are part of the full drive.
        let full_drive = parent_inputs_moved.any()
            || font_inputs_moved
            || environment_moved_under_substitutions
            || scratch.document_environment_moved
            || scratch.recompute_in_full
            || delta.properties().iter().any(|&property| {
                !property_computes_in_remaining_phase(property) || property_feeds_box_type_transformation(property)
            });
        let delta_property_count = delta.properties().len() as u64;
        // A delta that moves a longhand declaring the element's CSS transitions is a row whose only
        // remaining obligation is the transition step, and the host can run that step after the
        // batch: it needs the style the row moved away from, which the published delta names, and
        // the style it moved to, which is the record the host installs. The record the delta starts
        // at is asked to hold no animation of its own - which is what the record's animation state
        // is asked about here, not somewhere else's - so the step decides over transitions alone,
        // and nothing the element already holds can be cancelled.
        //
        // When the delta moves nothing else, every other group is copied from the record the delta
        // starts at, so no value a transition runs on moved either: the registration is the whole
        // of the step, and the host takes the cheaper of the two drains.
        let owes_a_transition_step = !full_drive
            && delta
                .properties()
                .iter()
                .any(|&property| longhand_only_declares_a_css_transition(property))
            && !self.record_requires_cpp_animation(old_style_record);
        let owes_a_transition_registration = owes_a_transition_step.then(|| {
            delta
                .properties()
                .iter()
                .all(|&property| longhand_only_declares_a_css_transition(property))
        });
        // A delta that moves a longhand declaring the element's CSS animations is a row whose only
        // remaining obligation is the animation plan, and the host can apply that after the batch:
        // the plan is a function of the longhands this drive computes, the `@keyframes` the host
        // published before the stage began, and the animations the element already holds.
        //
        // It is taken only where the element holds none - which is where the batch let the row this
        // far at all, since an element with an animation of its own is refused before its winners
        // are compared - so the plan can do nothing but start what its definitions name: there is
        // nothing to match, nothing to retime, nothing to cancel. A delta that also moves a
        // transition declaration is left to C++, which decides both in one computation.
        let owes_an_animation_plan = delta
            .properties()
            .iter()
            .any(|&property| longhand_declares_a_css_animation(property))
            && !delta
                .properties()
                .iter()
                .any(|&property| longhand_only_declares_a_css_transition(property))
            && self
                .element_css_defined_animations(node, animations::ELEMENT_ANIMATION_SLOT)
                .is_empty()
            && self.animation_keyframes().only_the_document_scope_defines_keyframes();
        // Partial drives can share across parents whose inherited inputs agree. Keep the full
        // parent record in the key when a non-inherited property explicitly inherits, including
        // through substitution, or when a full drive may read more of the parent's style.
        let parent = self.tree.flat_tree_parent(node);
        let parent_record = parent.and_then(|parent| self.computed_group_sets.assigned_style_record(parent));
        let mut cohort_parent = RecordDeltaParent::Exact(parent_record.map_or(0, |record| record.raw()));
        if !full_drive
            && let (Some(parent), Some(parent_record)) = (parent, parent_record)
            && !self.state_has_substitutions(node, state)
            && let Some(inputs) = self.cold_record_parent(node, parent, parent_record, state)
        {
            cohort_parent = RecordDeltaParent::Inputs(inputs);
        }
        let cohort = (
            old_style_record.raw(),
            state,
            facts,
            cohort_parent,
            environment.unwrap_or(0),
            RootFontInputs::from_document(&inputs),
            self.monospace_cohort_key(node, state),
        );
        if let Some(&(new_style_record, cohort_explicitly_inherited_groups)) = scratch.cohorts.get(&cohort) {
            // The row takes another node's record whole, so its plan is decided from that record's
            // own longhands rather than from a drive of this node's.
            let animation_plan = match owes_an_animation_plan {
                true => match self.settled_animation_plan_from_record(node, new_style_record) {
                    Some(plan) => Some(plan),
                    None => {
                        counters.bump(Counter::EngineComputedRecordBailProperty);
                        return None;
                    }
                },
                false => None,
            };
            self.note_node_substitution(node, scratch, state, current_environment);
            let delta =
                self.computed_group_sets
                    .assign_engine_computed_record(node, old_style_record, new_style_record)?;
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
            if let Some(registration_only) = owes_a_transition_registration {
                self.nodes_owing_a_transition_registration
                    .insert(node, registration_only);
            }
            if let Some(plan) = animation_plan {
                self.nodes_owing_animation_definitions.insert(node, plan);
            }
            return Some(delta);
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
        let (writing_mode, direction) = {
            let Some(view) = self.computed_group_sets.style_record_view(old_style_record.raw()) else {
                counters.bump(Counter::EngineComputedRecordBailRecord);
                return None;
            };
            let inherited_box = unsafe {
                view.payloads[crate::css::computed_value_types::STYLE_GROUP_INDEX_INHERITED_BOX]
                    .cast::<crate::css::computed_values::InheritedBoxValues>()
                    .deref()
            };
            (inherited_box.writing_mode, inherited_box.direction)
        };
        for &property in delta.properties() {
            // The counter-style environment behind `content` and `list-style-type` is resolved in
            // the C++ computation. A delta that moves the element's transition declarations owes
            // the host the transition step, and one that moves its animation declarations owes the
            // host the animation plan; both ride out of the batch as effects of the row.
            if property_starts_animation_or_counter_environment(property)
                && !(owes_a_transition_step && longhand_only_declares_a_css_transition(property))
                && !(owes_an_animation_plan && longhand_declares_a_css_animation(property))
                && !self.counter_environment_winner_keeps_the_record(state, old_style_record, property)
            {
                counters.bump(Counter::EngineComputedRecordBailProperty);
                return None;
            }
            let groups = match computed_group_dependency_mask(property) {
                Some(groups) => groups,
                // A longhand the font resolution selects by feeds no group of its own; the full
                // drive it takes rebuilds every group.
                None if full_drive && font_resolution_selects_by(property) => 0,
                None => {
                    counters.bump(Counter::EngineComputedRecordBailProperty);
                    return None;
                }
            };
            groups_to_rebuild |= groups;
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
                let Some(groups) = computed_group_dependency_mask(counterpart) else {
                    counters.bump(Counter::EngineComputedRecordBailProperty);
                    return None;
                };
                groups_to_rebuild |= groups;
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
            let Some(dependencies) = self.computed_group_sets.current_color_dependency_mask(target) else {
                counters.bump(Counter::EngineComputedRecordBailRecord);
                return None;
            };
            groups_to_rebuild |= dependencies;
            // The dependents compute again from their specified values, so the table spells them
            // the way a fresh computation does, not the way an inherited-group swap resolved them.
            let Some(dependent_properties) = self.computed_group_sets.current_color_dependency_properties(target)
            else {
                counters.bump(Counter::EngineComputedRecordBailRecord);
                return None;
            };
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
        if full_drive {
            groups_to_rebuild = (1 << crate::css::table_group_builder::group_index::COUNT) - 1;
        } else if groups_to_rebuild & (1 << STYLE_GROUP_INDEX_FONT) != 0 {
            counters.bump(Counter::EngineComputedRecordBailFontPhase);
            return None;
        }

        if goal == FontDriveGoal::RootInputs && !full_drive {
            // NB: No font property moved, but borrowing the retained font still needs the
            //     proof that only the named rule flips changed the computation's inputs.
            if exact_flipped_rules.is_some() {
                scratch.font_drive.root_inputs = self.root_font_inputs_from_record(old_style_record);
            } else {
                scratch.font_drive.root_inputs_unproven = true;
            }
            return None;
        }

        let store = match scratch.stores.get(&(state, current_environment)) {
            Some(store) => store.clone(),
            None => {
                let mut substituted = false;
                let store = std::sync::Arc::new(self.cascaded_store_for_state(
                    node,
                    state,
                    None,
                    current_environment,
                    &mut substituted,
                    counters,
                )?);
                scratch.store_capacity_bytes += store.capacity_bytes();
                scratch.stores.insert((state, current_environment), store.clone());
                if substituted {
                    scratch.substituted_states.insert((state, current_environment));
                }
                store
            }
        };
        self.note_node_substitution(node, scratch, state, current_environment);
        let mut driver_input_moved = false;
        let mut explicitly_inherited_groups = 0;
        let partial = if full_drive {
            None
        } else {
            let partial = self.engine_driven_table(
                node,
                old_style_record,
                &store,
                &selected,
                &inputs,
                &mut driver_input_moved,
                &mut explicitly_inherited_groups,
                counters,
            );
            if partial.is_none() && !driver_input_moved {
                return None;
            }
            partial
        };
        let (table, length, longhand_evaluations, font) = match partial {
            Some(partial) => partial,
            // A partial drive whose driver inputs moved reaches values it did not select, so the
            // record is driven in full and every group is rebuilt.
            None => {
                if driver_input_moved {
                    groups_to_rebuild = (1 << crate::css::table_group_builder::group_index::COUNT) - 1;
                }
                let subject = self.element_drive_subject(node, counters)?;
                self.engine_full_drive(
                    subject,
                    Some(old_style_record),
                    &store,
                    &inputs,
                    &mut scratch.font_drive,
                    goal,
                    &mut explicitly_inherited_groups,
                    counters,
                )?
            }
        };
        // The plan is decided from the longhands this drive computed, before the table goes into
        // the record.
        let animation_plan = owes_an_animation_plan.then(|| self.settled_animation_plan(node, &table));
        let parent_in_display_none_subtree = self
            .tree
            .flat_tree_parent(node)
            .and_then(|parent| self.computed_group_sets.assigned_style_record(parent))
            .and_then(|record| self.computed_group_sets.style_record_view(record.raw()))
            .is_some_and(|view| view.dependency_flags & (1 << 2) != 0);
        let Some(assembly) = self.computed_group_sets.replace_engine_computed_table(
            node,
            old_style_record,
            old_style_record,
            table,
            groups_to_rebuild,
            &length,
            font.as_ref(),
            parent_in_display_none_subtree,
            environment,
        ) else {
            counters.bump(Counter::EngineComputedRecordBailAssemble);
            return None;
        };
        self.settle_computed_memory();
        counters.add(
            Counter::ComputedOutputGroupsCanonicalized,
            u64::from(assembly.canonicalized_groups),
        );
        if assembly.group_set_unchanged {
            counters.bump(Counter::ComputedWinnerPropagationStops);
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
        if !driver_input_moved {
            scratch.cohorts.insert(cohort, (delta.1, explicitly_inherited_groups));
        }
        if explicitly_inherited_groups != 0 {
            self.nodes_owing_explicit_inheritance
                .insert(node, explicitly_inherited_groups);
        }
        // A partial drive whose driver inputs moved was driven in full instead, so values the
        // delta does not name may have moved too: the host runs the whole step for such a row.
        if let Some(registration_only) = owes_a_transition_registration {
            self.nodes_owing_a_transition_registration
                .insert(node, registration_only && !driver_input_moved);
        }
        if let Some(plan) = animation_plan {
            self.nodes_owing_animation_definitions.insert(node, plan);
        }
        if derived_beneath_a_composition {
            self.nodes_owing_an_animation_sample.insert(node);
        }
        Some(delta)
    }

    /// The animation plan a settled row leaves for the host, decided from one longhand table.
    fn settled_animation_plan(
        &self,
        node: StyleNodeID,
        table: &ComputedLonghandTable,
    ) -> animations::SettledAnimationPlan {
        crate::css::style_compute::build_settled_animation_plan(
            table,
            self.element_css_defined_animations(node, animations::ELEMENT_ANIMATION_SLOT),
            self.animation_keyframes(),
            self.tree.tree_scope(node),
        )
    }

    /// The same, for a row that takes another node's record whole: the plan is decided from the
    /// longhands that record carries.
    fn settled_animation_plan_from_record(
        &self,
        node: StyleNodeID,
        style_record: computed::FinalStyleRecordID,
    ) -> Option<animations::SettledAnimationPlan> {
        let view = self.computed_group_sets.style_record_view(style_record.raw())?;
        // SAFETY: A record's table outlives the view the assignment below takes it from.
        let table = unsafe { view.longhand_table.as_ref() }?;
        Some(self.settled_animation_plan(node, table))
    }

    /// The animation definitions the engine-computed record the host is about to install for this
    /// node leaves to be applied after the batch, taking the debt with the answer so that exactly
    /// one application drains it. The plan stays alive until the next one is taken, which is long
    /// enough for the host to read it.
    pub(crate) fn take_settled_animation_definitions(
        &mut self,
        node: StyleNodeID,
    ) -> Option<&animations::SettledAnimationPlan> {
        self.animation_definitions_being_applied = self.nodes_owing_animation_definitions.remove(&node);
        self.animation_definitions_being_applied.as_ref()
    }

    /// Account for a record the engine derived and leave its commitment to C++'s acknowledgement.
    /// Move a node's record to the environment C++ refreshed its inherited custom-property data
    /// to, without recomputing anything: what an inherited-custom-properties reaction C++ settled
    /// by refreshing the data alone publishes. The new record's identity, or nothing when the node
    /// holds no base record to move.
    /// The style groups an engine-computed record read straight from the parent through an
    /// explicit `inherit`, taken with the answer so that exactly one application marks the parent.
    pub(crate) fn take_explicit_inheritance_debt(&mut self, node: StyleNodeID) -> u32 {
        self.nodes_owing_explicit_inheritance.remove(&node).unwrap_or(0)
    }

    /// What the engine-computed record the host is about to install for this node leaves to be
    /// applied after the batch, taking the transition debt with the answer so that exactly one
    /// application drains it: the low two bits are the transition step - 0 nothing, 1 the
    /// registration alone, 2 the whole step - and `OWES_AN_ANIMATION_PLAN` says the row also left
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
        let plan = match self.nodes_owing_animation_definitions.contains_key(&node) {
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
    pub(crate) fn acknowledge_engine_computed_record(&mut self, node: StyleNodeID, counters: &mut Counters) {
        if let Some(pending_records) = self.engine_computed_records_pending.remove(&node) {
            for pending in pending_records {
                let target = computed::ComputedStyleTarget::new(node, pending.pseudo_kind);
                self.remove_pending_style_computation_selection(target);
                // A pseudo-element settled as gone is removed now that C++ has cleared its style.
                if pending.pseudo_kind != u8::MAX && pending.new_style_record == computed::FinalStyleRecordID::NONE {
                    self.remove_computed_pseudo(node, pending.pseudo_kind, counters);
                    continue;
                }
                self.computed_group_sets.take_pending_cascade_state(target);
                if let Some(cascade_state) = pending.cascade_state {
                    self.computed_group_sets.bind_cascade_state(target, cascade_state);
                }
                // The record is installed and no C++ computation left an input record for it. Leave
                // what that computation would have left instead, so the element's next one selects
                // what it rebuilds rather than rebuilding every group.
                if pending.pseudo_kind == u8::MAX {
                    self.remember_settled_computation_context(node);
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
    ) -> Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)> {
        // A first record whose winners declare CSS animations owes the host the plan that starts
        // them, the way a warm row does: the element holds none yet, so every definition starts one.
        let owes_an_animation_plan = self.cold_record_owes_an_animation_plan(node, cascade_state.1);
        let delta =
            self.engine_cold_record_impl(node, cascade_state, scratch, goal, counters, owes_an_animation_plan)?;
        if owes_an_animation_plan {
            // The plan is decided from the record the row installs, which carries the longhands the
            // drive computed. A record without one is no record to settle a plan against.
            let Some(plan) = self.settled_animation_plan_from_record(node, delta.1) else {
                counters.bump(Counter::EngineComputedRecordBailProperty);
                return None;
            };
            self.nodes_owing_animation_definitions.insert(node, plan);
        }
        Some(delta)
    }

    /// Whether a first record for this node would owe the host an animation plan: its winners
    /// declare a CSS animation and nothing else the C++ computation has to decide, the element
    /// holds no animation yet, and the document's own scope is the only one defining `@keyframes`.
    fn cold_record_owes_an_animation_plan(&self, node: StyleNodeID, state: CascadeStateID) -> bool {
        let mut declares_an_animation = false;
        for property in self.winner_groups.semantic_delta_properties(None, state) {
            if longhand_declares_a_css_animation(property) {
                declares_an_animation = true;
                continue;
            }
            // Anything else the first-record gate refuses is still the C++ computation's, and a
            // transition declaration beside an animation one is decided with it.
            if self.first_record_winner_needs_cpp(state, property) {
                return false;
            }
        }
        declares_an_animation
            && self
                .element_css_defined_animations(node, animations::ELEMENT_ANIMATION_SLOT)
                .is_empty()
            && self.animation_keyframes().a_first_record_may_start_an_animation()
    }

    #[allow(clippy::too_many_arguments)]
    fn engine_cold_record_impl(
        &mut self,
        node: StyleNodeID,
        cascade_state: (u64, CascadeStateID),
        scratch: &mut EngineComputedRecordScratch,
        goal: FontDriveGoal,
        counters: &mut Counters,
        owes_an_animation_plan: bool,
    ) -> Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)> {
        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        let (_, state) = cascade_state;
        let Some(mut inputs) = self.document_style_computation_inputs else {
            counters.bump(Counter::EngineComputedRecordBailNoEnvironment);
            return None;
        };
        if let Some((root, root_inputs)) = scratch.root_element_inputs
            && root == node
        {
            root_inputs.apply_to(&mut inputs);
        }
        let facts = self.computed_group_sets.adjustment_facts(node);
        let mut explicitly_inherited_groups = 0;
        let parent = self.tree.flat_tree_parent(node);
        // Only the document element is styled without a flat-tree parent: it inherits from the
        // initial values.
        if parent.is_none() && facts & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT == 0 {
            counters.bump(Counter::EngineComputedRecordBailRecordParent);
            return None;
        }
        let parent_record = match parent {
            Some(parent) => match self.computed_group_sets.assigned_style_record(parent) {
                Some(parent_record) => Some(parent_record),
                None => {
                    counters.bump(Counter::EngineComputedRecordBailRecordParent);
                    return None;
                }
            },
            None => None,
        };
        // The document element's environment is its own, which is nothing without declarations;
        // any other node's is its declarations resolved over the parent's.
        let Some(parent_environment) = parent.map_or(Some(0), |parent| {
            self.computed_group_sets.custom_property_environment_identity(parent)
        }) else {
            counters.bump(Counter::EngineComputedRecordBailRecordParent);
            return None;
        };
        let Some(pseudo_styles) = self.pseudo_style_mask(node) else {
            counters.bump(Counter::EngineComputedRecordBailWinner);
            return None;
        };
        let cache_key = parent
            .zip(parent_record)
            .and_then(|(parent, parent_record)| self.cold_record_parent(node, parent, parent_record, state))
            .map(|parent| ColdRecordKey {
                monospace_recascaded_font_size: self.monospace_cohort_key(node, state),
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
        if !self.node_declares_custom_properties(node)
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
            return Some(delta);
        }
        // A winner that starts an animation or reads the counter-style environment keeps the
        // record in C++. The font-phase longhands feed no group of their own: the full drive
        // resolves the font from them and rebuilds every group, rejecting the values the font
        // resolution does not pass on yet.
        for property in self.winner_groups.semantic_delta_properties(None, state) {
            if self.first_record_winner_needs_cpp(state, property)
                && !(owes_an_animation_plan && longhand_declares_a_css_animation(property))
            {
                counters.bump(Counter::EngineComputedRecordBailProperty);
                return None;
            }
        }
        let Some(environment) = self.engine_custom_property_environment(node, parent_environment, &inputs, counters)
        else {
            counters.bump(Counter::EngineComputedRecordBailCustomProperties);
            return None;
        };
        // The state has to be one the engine can compute from before any record is shared under
        // it: a record C++ computed for a per-element value, such as a `random()` draw, is that
        // element's alone. A store with substituted values is the environment's as well as the
        // state's, and admits nothing for the state alone.
        let store = match scratch.stores.get(&(state, environment)) {
            Some(store) => store.clone(),
            None => {
                let mut substituted = false;
                let store = self.cascaded_store_for_state(node, state, None, environment, &mut substituted, counters);
                scratch.computability.remember(
                    (
                        node,
                        cascade_state.0,
                        state,
                        environment,
                        inputs.custom_property_registration_generation,
                    ),
                    store.is_some(),
                );
                let store = std::sync::Arc::new(store?);
                scratch.store_capacity_bytes += store.capacity_bytes();
                scratch.stores.insert((state, environment), store.clone());
                if substituted {
                    scratch.substituted_states.insert((state, environment));
                }
                store
            }
        };
        self.note_node_substitution(node, scratch, state, environment);
        let cache_key = parent
            .zip(parent_record)
            .and_then(|(parent, parent_record)| self.cold_record_parent(node, parent, parent_record, state))
            .map(|parent| ColdRecordKey {
                monospace_recascaded_font_size: self.monospace_cohort_key(node, state),
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
            return Some(delta);
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
                && let Some((table, length, longhand_evaluations, _)) = self.engine_driven_table(
                    node,
                    donor.record.record,
                    &store,
                    &selected,
                    &inputs,
                    &mut false,
                    &mut explicitly_inherited_groups,
                    counters,
                )
            {
                let parent_in_display_none_subtree = parent_record
                    .and_then(|record| self.computed_group_sets.style_record_view(record.raw()))
                    .is_some_and(|view| view.dependency_flags & (1 << 2) != 0);
                if let Some(assembly) = self.computed_group_sets.replace_engine_computed_table(
                    node,
                    donor.record.record,
                    computed::FinalStyleRecordID::NONE,
                    table,
                    groups_to_rebuild,
                    &length,
                    None,
                    parent_in_display_none_subtree,
                    Some(environment),
                ) {
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
                    if let Some(cache_key) = cache_key {
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
                    return Some(assembly.delta);
                }
            }
        }
        let subject = DriveSubject {
            target,
            recascade_node: Some(node),
            parent,
            facts,
        };
        let (table, length, longhand_evaluations, font) = self.engine_full_drive(
            subject,
            None,
            &store,
            &inputs,
            &mut scratch.font_drive,
            goal,
            &mut explicitly_inherited_groups,
            counters,
        )?;
        let font = font.expect("a full drive resolves the font");
        let (new_style_record, swap_eligible) = self.assemble_and_publish_engine_record(
            target,
            parent_record,
            table,
            &length,
            &font,
            environment,
            pseudo_styles,
            0,
            Some(cascade_state),
            &mut scratch.computability,
            counters,
        )?;
        let delta = (computed::FinalStyleRecordID::NONE, new_style_record);
        // The publication itself kept the record for later transactions; alike elements in this
        // one take it from the cohort.
        if let Some(cache_key) = cache_key {
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
        Some(delta)
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
        // A record with transitions will need C++ on its next change. Keep its initial
        // computation in C++ too, so that fallback retains the input record and can select
        // only the changed groups instead of rebuilding the entire style.
        if self.record_requires_cpp_animation(record) {
            counters.bump(Counter::EngineComputedRecordBailRecordOverlay);
            return None;
        }
        if !self.engine_pseudo_inputs_available(node, Some(record), counters) {
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

    /// Whether a first record's winner keeps the record's computation in C++: a property that
    /// starts an animation or transition, or reads the counter-style environment. A
    /// `list-style-type` reads it only through an overridable counter-style name. The font-phase
    /// longhands without a group of their own are inputs of the font group the full drive builds.
    fn first_record_winner_needs_cpp(&self, state: CascadeStateID, property: u16) -> bool {
        use crate::css::property_metadata::property_id as prop;
        if property == prop::LIST_STYLE_TYPE {
            return self.list_style_type_winner_reads_counter_style_environment(state);
        }
        // A `content` that names no counter reads no counter-style environment, and a first record
        // that needs none is published without one. The node's own custom properties are the other
        // half of the question the warm gate asks, and it asks it of the same two properties.
        if property == prop::CONTENT {
            return !self
                .winner_groups
                .winner_in_state(state, prop::CONTENT)
                .and_then(|winner| self.winner_groups.resolved_winner(winner))
                .is_some_and(|winner| match self.specified_values.value(winner.key.value) {
                    Lookup::Known(value) => content_value_is_engine_computable(value),
                    _ => false,
                });
        }
        if property == prop::DISPLAY {
            // A list item's marker is derived beside its first record, and the default marker's
            // font is not one the engine resolves yet: the record would be derived and abandoned.
            return self.display_winner_is_list_item(state);
        }
        // An anchor name is one the host registers from whichever record it installs, a first
        // record included: `Element::update_anchor_name_registry` runs on that install too.
        property_starts_animation_or_counter_environment(property)
            || (computed_group_dependency_mask(property).is_none() && !font_group_carries_longhand(property))
    }

    /// Whether a moved `content` or `list-style-type` winner leaves the record's counter-style
    /// environment where it is: the record it moves away from names none, and the winner's value
    /// reads none, so the identity the engine copies from the old record stays right. C++ names the
    /// environment on a record only when a named counter style has to be resolved against it.
    ///
    /// A node declaring custom properties is left to C++ with such a winner: the environment the
    /// engine would name for it may be one the host resolved and no longer installs, and the host
    /// then computes the element and its children over again.
    fn counter_environment_winner_keeps_the_record(
        &self,
        state: CascadeStateID,
        old_style_record: computed::FinalStyleRecordID,
        property: u16,
    ) -> bool {
        use crate::css::property_metadata::property_id as prop;
        if property != prop::CONTENT && property != prop::LIST_STYLE_TYPE {
            return false;
        }
        if self
            .computed_group_sets
            .style_record_view(old_style_record.raw())
            .is_none_or(|view| view.counter_style_environment_identity != 0)
        {
            return false;
        }
        if property == prop::LIST_STYLE_TYPE {
            return !self.list_style_type_winner_reads_counter_style_environment(state);
        }
        self.winner_groups
            .winner_in_state(state, prop::CONTENT)
            .and_then(|winner| self.winner_groups.resolved_winner(winner))
            .is_some_and(|winner| match self.specified_values.value(winner.key.value) {
                Lookup::Known(value) => content_value_is_engine_computable(value),
                _ => false,
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

    /// The font size the monospace recascade gives a node, walking the cascaded font-size of every
    /// ancestor from a 13px default the way `recascade_font_size_if_needed` does. `None` when the
    /// walk needs a resolution context only C++ can supply, or when its answer would depend on the
    /// viewport, which C++ records on the element beside the size.
    pub(super) fn monospace_recascaded_font_size(&self, node: StyleNodeID) -> Option<i32> {
        use crate::css::style_compute::{FontSizeRecascadeStatus, recascade_font_size_batch};

        let inputs = self.document_style_computation_inputs?;
        let default_size = crate::css::css_pixels::CssPixels::from_integer(13).raw_value();
        let records =
            self.retained_inheritance_ancestor_style_records(node, crate::css::cascaded_properties::NO_PSEUDO_ELEMENT);
        let batch = recascade_font_size_batch(
            records.len(),
            |index| {
                let record = records[index];
                if record == 0 {
                    return std::ptr::null();
                }
                self.style_record_view(record)
                    .and_then(|view| unsafe { view.longhand_table.as_ref() })
                    .map_or(std::ptr::null(), ComputedLonghandTable::raw_cascaded_font_size)
            },
            0,
            default_size,
            false,
            default_size,
            crate::css::style_compute::FontSizeRecascadeDocumentInputs {
                root_font_size: inputs.root_font_size,
                root_font_metrics_depend_on_viewport_metrics: inputs.root_font_metrics_depend_on_viewport_metrics,
                viewport_width: inputs.viewport_width,
                viewport_height: inputs.viewport_height,
            },
            std::ptr::null(),
        );
        (batch.status == FontSizeRecascadeStatus::Complete
            && !batch.depends_on_viewport_metrics
            && !batch.skipped_calculated_value)
            .then_some(batch.current_size_raw)
    }

    /// What a record for this node under this cascade state owes the monospace recascade, as a
    /// cohort key carries it. Two nodes whose parents hold equal records can still sit under
    /// different cascaded font-size chains, and the recascade reads the chain rather than the
    /// records, so a cohort that ignored this would hand one node the other's font size.
    fn monospace_cohort_key(&self, node: StyleNodeID, state: CascadeStateID) -> i32 {
        if !self.font_family_winner_is_monospace(state) {
            return 0;
        }
        self.monospace_recascaded_font_size(node).unwrap_or(i32::MIN)
    }

    fn display_winner_is_list_item(&self, state: CascadeStateID) -> bool {
        use crate::css::style_value::StyleValueData;
        self.winner_groups
            .winner_in_state(state, crate::css::property_metadata::property_id::DISPLAY)
            .and_then(|winner| self.winner_groups.resolved_winner(winner))
            .is_some_and(|winner| match self.specified_values.value(winner.key.value) {
                Lookup::Known(StyleValueData::Display { raw }) => {
                    crate::css::display::FfiDisplay::from_raw(*raw).is_list_item()
                }
                _ => true,
            })
    }

    fn list_style_type_winner_reads_counter_style_environment(&self, state: CascadeStateID) -> bool {
        let Some(winner) = self
            .winner_groups
            .winner_in_state(state, crate::css::property_metadata::property_id::LIST_STYLE_TYPE)
            .and_then(|winner| self.winner_groups.resolved_winner(winner))
        else {
            return true;
        };
        self.list_style_type_value_reads_counter_style_environment(&winner)
    }

    /// Whether a `list-style-type` winner names a counter style the environment may define: an
    /// overridable name. `none`, a string, `symbols()` and the non-overridable names need none.
    fn list_style_type_value_reads_counter_style_environment(&self, winner: &PropertyWinner) -> bool {
        use crate::css::style_value::StyleValueData;
        match self.specified_values.value(winner.key.value) {
            Lookup::Known(StyleValueData::CounterStyle { is_symbols, name, .. }) => {
                !*is_symbols && !counter_style_name_is_non_overridable(name.units())
            }
            Lookup::Known(StyleValueData::Keyword { keyword }) => *keyword != crate::css::style_compute::keyword::NONE,
            Lookup::Known(StyleValueData::String { .. }) => false,
            _ => true,
        }
    }

    /// Whether a pseudo-element's winner keeps its record in C++: the same rule as a first
    /// record's, since a pseudo-element record the engine settles is computed in full.
    fn pseudo_winner_needs_cpp(&self, winner: &PropertyWinner) -> bool {
        use crate::css::property_metadata::property_id as prop;
        if winner.property == prop::LIST_STYLE_TYPE {
            return self.list_style_type_value_reads_counter_style_environment(winner);
        }
        winner.property == prop::ANCHOR_NAME || property_starts_animation_or_counter_environment(winner.property)
    }

    fn record_requires_cpp_animation(&self, record: computed::FinalStyleRecordID) -> bool {
        self.computed_group_sets
            .style_record_view(record.raw())
            .is_none_or(|view| {
                !view.animated_overlay.is_null()
                    || (unsafe { view.longhand_table.as_ref() })
                        .is_none_or(crate::css::style_compute::has_active_transition_properties)
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
            if property_starts_animation_or_counter_environment(property) {
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

    fn retry_engine_record_after_ancestor_step(
        &mut self,
        node: StyleNodeID,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> u64 {
        let facts = self.computed_group_sets.adjustment_facts(node);
        if facts & bridge::element_adjustment_fact::DISALLOW_DISPLAY_CONTENTS != 0 {
            return 0;
        }
        let Lookup::Known(cascade_state) = self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        else {
            return 0;
        };
        if !scratch.font_drive.is_pending()
            && !self.engine_pseudo_inputs_available(
                node,
                self.computed_group_sets.assigned_style_record(node),
                counters,
            )
        {
            return 0;
        }
        if !scratch.font_drive.is_pending()
            && !self.node_declares_custom_properties(node)
            && let Some(old_style_record) = self.computed_group_sets.assigned_style_record(node)
            && let Some(inputs) = self.document_style_computation_inputs
            && let Some(parent) = self.tree.flat_tree_parent(node)
            && let Some(parent_record) = self.computed_group_sets.assigned_style_record(parent)
            && let Some(environment) = self.computed_group_sets.custom_property_environment_identity(parent)
            && let Some(pseudo_styles) = self.pseudo_style_mask(node)
        {
            let cache_key = self
                .cold_record_parent(node, parent, parent_record, cascade_state.1)
                .map(|parent| ColdRecordKey {
                    monospace_recascaded_font_size: self.monospace_cohort_key(node, cascade_state.1),
                    parent,
                    previous_style_record: old_style_record.raw(),
                    generation: cascade_state.0,
                    state: cascade_state.1,
                    facts,
                    pseudo_styles,
                    environment,
                    font_environment_generation: inputs.font_environment_generation,
                    root_font_inputs: RootFontInputs::from_document(&inputs),
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
                let pseudos_settled = self
                    .engine_pseudo_records(node, Some(old_record), record, cascade_state.0, scratch, counters)
                    .is_some();
                if pseudos_settled && scratch.pseudo_uses_substitution {
                    scratch.substitution_effects.push((node, true));
                }
                if !pseudos_settled {
                    // A pseudo-element the engine cannot settle sends the element to C++.
                    if scratch.font_drive.request.is_some() {
                        scratch.pending_element = Some((old_record, record));
                    } else {
                        counters.bump(Counter::RetryAfterAncestorPseudoAbandons);
                        self.abandon_engine_computed_record(node, scratch, counters);
                    }
                    self.apply_substitution_effects(scratch);
                    return 0;
                }
                counters.bump(Counter::RetryAfterAncestorColdHits);
                self.apply_substitution_effects(scratch);
                return record.raw();
            }
        }
        if !scratch.font_drive.is_pending() && self.computed_group_sets.node_answer_is_incomplete(node) {
            return 0;
        }
        let cascade_winners_are_complete = self
            .current_published_answer(node)
            .is_some_and(|answer| answer.cascade_winners_are_complete);
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
        let Some((_, record)) = record else {
            return 0;
        };
        record.raw()
    }

    /// Build a driven table's groups against the parent record's payloads and publish the record
    /// for `target` the way a C++ computation publishes one; the record's swap eligibility comes
    /// back beside its identity.
    #[allow(clippy::too_many_arguments)]
    fn assemble_and_publish_engine_record(
        &mut self,
        target: computed::ComputedStyleTarget,
        parent_record: Option<computed::FinalStyleRecordID>,
        mut table: ComputedLonghandTable,
        length: &crate::css::style_compute::FfiLengthResolutionContext,
        font: &crate::css::table_group_builder::FfiFontGroupBuildInputs,
        environment: u64,
        pseudo_styles: u64,
        counter_style_environment_identity: u64,
        cascade_state: Option<(u64, CascadeStateID)>,
        scratch: &mut EngineComputabilityScratch,
        counters: &mut Counters,
    ) -> Option<(computed::FinalStyleRecordID, bool)> {
        use crate::css::computed_value_types::STYLE_GROUP_INDEX_FONT;
        use crate::css::table_group_builder::group_index;

        // The document element's groups build against no parent payloads.
        let (parent_payloads, parent_in_display_none_subtree) = match parent_record {
            Some(parent_record) => {
                let Some(parent_view) = self.computed_group_sets.style_record_view(parent_record.raw()) else {
                    counters.bump(Counter::EngineComputedRecordBailRecordParent);
                    return None;
                };
                (parent_view.payloads, parent_view.dependency_flags & (1 << 2) != 0)
            }
            None => (&[SharedPayload::null(); group_index::COUNT][..], false),
        };
        let Ok(used_color_scheme) = u8::try_from(table.effective_color_scheme()) else {
            counters.bump(Counter::EngineComputedRecordBailDrive);
            return None;
        };
        let display_is_none = crate::css::style_compute::effective_display(&table, None).is_none();
        table.set_in_display_none_subtree(parent_in_display_none_subtree || display_is_none);
        table.freeze();
        let swap_eligible = table.property_inheritance_is_standard()
            && !table.display_is_list_item()
            && !crate::css::style_compute::has_active_transition_properties(&table);
        let table = table.into_raw_shared();
        let release_table = |table: *const ComputedLonghandTable| unsafe {
            crate::css::computed_longhand_table::rust_computed_longhand_table_release(table.cast_mut());
        };
        let Some(current_color) =
            crate::css::table_group_builder::own_color_from_table(unsafe { &*table }, used_color_scheme, Some(length))
        else {
            release_table(table);
            counters.bump(Counter::EngineComputedRecordBailAssemble);
            return None;
        };
        let mut payloads = Vec::with_capacity(group_index::COUNT);
        for (group, &parent_payload) in parent_payloads.iter().enumerate().take(group_index::COUNT) {
            let payload = if group == STYLE_GROUP_INDEX_FONT {
                unsafe {
                    crate::css::table_group_builder::rebuild_font_group_from_table(
                        &*table,
                        font,
                        parent_payload.as_ptr(),
                    )
                }
            } else {
                unsafe {
                    crate::css::table_group_builder::rebuild_group_from_table(
                        &*table,
                        group,
                        parent_payload.as_ptr(),
                        current_color,
                        used_color_scheme,
                        Some(length),
                    )
                }
            };
            let Some(payload) = payload.map(SharedPayload::new) else {
                for (group, payload) in payloads.into_iter().enumerate() {
                    crate::css::computed_values::release_group_payload(group, SharedPayload::as_ptr(payload));
                }
                release_table(table);
                counters.bump(Counter::EngineComputedRecordBailAssemble);
                return None;
            };
            payloads.push(payload);
        }
        let holds_image_values = crate::css::computed_values::style_group_payloads_hold_image_values(
            HostShared::as_pointer_slice(&payloads),
        );
        let dependency_flags = unsafe { &*table }.publication_dependency_flags()
            | (u8::from(swap_eligible) * computed::INHERITED_GROUP_SWAP_ELIGIBLE)
            | (u8::from(holds_image_values) * computed::HOLDS_IMAGE_VALUES);
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
        let owned = computed::PendingRecordOwnership {
            groups: u32::try_from((1_u64 << payloads.len()) - 1).expect("a style group index fits the ownership mask"),
            table: true,
        };
        let publication = self.publish_computed_groups_impl(
            Some(target),
            &payloads,
            computed::ENGINE_INHERITED_GROUP_COUNT,
            environment,
            metadata_input,
            owned,
            scratch,
            counters,
        );
        let transferred = publication.transferred;
        for (group, payload) in payloads.into_iter().enumerate() {
            if transferred.groups & (1 << group) == 0 {
                crate::css::computed_values::release_group_payload(group, payload.as_ptr());
            }
        }
        if !transferred.table {
            release_table(table);
        }
        Some((publication.style_record_identity, swap_eligible))
    }

    /// A derivation that could not be completed: everything derived for `node` this flush goes
    /// back to what the node held, and nothing in the flush may share it.
    pub(super) fn abandon_engine_computed_record(
        &mut self,
        node: StyleNodeID,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) {
        for pending in self.engine_computed_records_pending.remove(&node).into_iter().flatten() {
            let derived = pending.new_style_record;
            if pending.pseudo_kind == u8::MAX {
                let target = computed::ComputedStyleTarget::new(node, u8::MAX);
                self.computed_group_sets.take_pending_cascade_state(target);
                self.computed_group_sets
                    .revert_engine_computed_record(node, derived, pending.old_style_record);
                scratch.cohorts.retain(|_, (record, _)| *record != derived);
                scratch.cold_cohorts.retain(|_, record| record.record != derived);
                self.engine_cold_record_cache
                    .retain(|_, record| record.record != derived);
                self.engine_cold_record_donors.retain(|_, donors| {
                    donors.retain(|donor| donor.record.record != derived);
                    !donors.is_empty()
                });
            } else {
                self.revert_engine_computed_pseudo_record(&pending, counters);
                scratch.pseudo_cohorts.retain(|_, record| *record != derived);
                self.engine_pseudo_record_cache.retain(|_, record| *record != derived);
            }
        }
        scratch.pseudo_deltas.clear();
        self.settle_computed_memory();
        counters.bump(Counter::EngineComputedRecordsAbandoned);
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
    /// declarations land in, and the ones holding colors when it declares `color`, which their
    /// currentcolor-dependent values resolve against.
    fn state_owned_inherited_groups(&self, state: CascadeStateID) -> u32 {
        use crate::css::computed_value_types::{
            STYLE_GROUP_INDEX_INHERITED_SVG, STYLE_GROUP_INDEX_INHERITED_TEXT, STYLE_GROUP_INDEX_INHERITED_UI,
        };
        use crate::css::property_metadata::{property_id as prop, property_style_group_index};
        self.winner_groups
            .properties_in_state(state)
            .fold(0_u32, |mask, property| {
                let mask = mask | property_style_group_index(property).map_or(0, |group| 1 << group);
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
        use crate::css::computed_value_types::STYLE_GROUP_INDEX_FONT;
        let view = self.computed_group_sets.style_record_view(record.raw())?;
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
        let Some(inputs) = self.document_style_computation_inputs.as_mut() else {
            return;
        };
        root_inputs.apply_to(inputs);
    }

    pub(crate) fn prepare_root_font_metrics_from_legacy(
        &mut self,
        node: StyleNodeID,
        table: &ComputedLonghandTable,
        font: &crate::css::table_group_builder::FfiFontGroupBuildInputs,
    ) {
        // Descendants in the same preorder batch resolve root-relative lengths before the host
        // installs this row. Publish the finalized row's exact font inputs at finalization time.
        if self.computed_group_sets.adjustment_facts(node) & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT == 0 {
            return;
        }
        let Some(inputs) = self.document_style_computation_inputs.as_mut() else {
            return;
        };
        RootFontInputs {
            metrics: [
                crate::css::css_pixels::CssPixels::from_raw(font.font_size_raw)
                    .to_double()
                    .to_bits(),
                f64::from(font.font_x_height).to_bits(),
                f64::from(font.font_ascent).to_bits(),
                f64::from(font.font_zero_advance).to_bits(),
                crate::css::css_pixels::CssPixels::from_raw(font.line_height_used_raw)
                    .to_double()
                    .to_bits(),
            ],
            depends_on_viewport: table.publication_dependency_flags() & (1 << 1) != 0,
        }
        .apply_to(inputs);
    }

    fn element_drive_subject(&mut self, node: StyleNodeID, counters: &mut Counters) -> Option<DriveSubject> {
        let facts = self.computed_group_sets.adjustment_facts(node);
        let parent = self.tree.flat_tree_parent(node);
        // Only the document element is styled without a flat-tree parent.
        if parent.is_none() && facts & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT == 0 {
            counters.bump(Counter::EngineComputedRecordBailRecordParent);
            return None;
        }
        if self.parent_composes_animations(node) {
            counters.bump(Counter::EngineComputedRecordBailRecordOverlay);
            return None;
        }
        Some(DriveSubject {
            target: computed::ComputedStyleTarget::new(node, u8::MAX),
            recascade_node: Some(node),
            parent,
            facts,
        })
    }

    /// Whether a node inherits values its parent's animations sample, which C++ composes over the
    /// parent's record. An animation that settles a custom property installs an environment of its
    /// own on the parent, and sampling moves it without a publication the engine sees.
    fn parent_composes_animations(&self, node: StyleNodeID) -> bool {
        self.tree.flat_tree_parent(node).is_some_and(|parent| {
            self.computed_group_sets.node_has_animation_overlay(parent)
                || self.computed_group_sets.adjustment_facts(parent) & bridge::element_adjustment_fact::HAS_ANIMATIONS
                    != 0
        })
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

    /// Whether a winner state is one the engine computes records from: every winner a plain rule
    /// declaration with a written value that needs no document context. Substituted declarations
    /// also depend on the custom-property environment and the registry used to parse them.
    fn state_is_engine_computable(
        &mut self,
        node: StyleNodeID,
        cascade_state: (u64, CascadeStateID),
        scratch: &mut EngineComputabilityScratch,
        counters: &mut Counters,
    ) -> bool {
        let environment = self
            .computed_group_sets
            .custom_property_environment_identity(node)
            .unwrap_or(0);
        let registration_generation = self
            .document_style_computation_inputs
            .map_or(0, |inputs| inputs.custom_property_registration_generation);
        let key = (
            node,
            cascade_state.0,
            cascade_state.1,
            environment,
            registration_generation,
        );
        if let Some(&admitted) = scratch.states.get(&key) {
            return admitted;
        }
        let mut substituted = false;
        let admitted = self
            .cascaded_store_for_state(node, cascade_state.1, None, environment, &mut substituted, counters)
            .is_some();
        scratch.remember(key, admitted);
        admitted
    }

    /// Whether C++ may publish one record as the answer for another element with this winner
    /// state. Values which read per-element or external context are never shared opaquely.
    fn state_is_opaque_record_shareable(
        &mut self,
        node: StyleNodeID,
        state: CascadeStateID,
        counters: &mut Counters,
    ) -> bool {
        for winner in self
            .winner_groups
            .winners_in_state(state)
            .filter_map(|winner| self.winner_groups.resolved_winner(winner))
        {
            match self.written_winner_value(node, &winner) {
                Ok(Some((_, _, checks))) if checks.whole_context_free => {}
                Err(counter) => {
                    counters.bump(counter);
                    return false;
                }
                _ => return false,
            }
        }
        true
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
        let mut ancestor = Some(parent);
        while let Some(current) = ancestor {
            let record = self.computed_group_sets.assigned_style_record(current)?;
            let view = self.computed_group_sets.style_record_view(record.raw())?;
            let table = unsafe { view.longhand_table.as_ref() }?;
            let display = crate::css::style_compute::effective_display(table, None);
            // C++ styles the children of a display:none element on demand, past the engine's
            // view of the parent, so no first record is computed under one.
            if display.is_none() {
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
    /// parent's inherited style and environment, a winner state the engine can compute from, the
    /// element facts and the pseudo-elements it has rules for.
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
        scratch: &mut EngineComputabilityScratch,
        counters: &mut Counters,
    ) {
        if target.is_pseudo() || !is_base_record {
            return;
        }
        let Some(inputs) = self.document_style_computation_inputs else {
            return;
        };
        let node = target.node();
        let facts = self.computed_group_sets.adjustment_facts(node);
        if self.node_declares_custom_properties(node) {
            return;
        }
        // A record C++ computed for an element with hints or animations is not what its winner
        // state alone describes.
        if facts
            & (bridge::element_adjustment_fact::HAS_PRESENTATIONAL_HINTS
                | bridge::element_adjustment_fact::HAS_ANIMATIONS)
            != 0
        {
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
        if !self.state_is_engine_computable(node, cascade_state, scratch, counters)
            && !self.state_is_opaque_record_shareable(node, cascade_state.1, counters)
        {
            return;
        }
        let Some(parent) = self.cold_record_parent(node, parent, parent_record, cascade_state.1) else {
            return;
        };
        let swap_eligible = self.computed_group_sets.node_inherited_group_swap_eligible(node);
        let key = ColdRecordKey {
            monospace_recascaded_font_size: self.monospace_cohort_key(node, cascade_state.1),
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
    ) -> Result<
        Option<(
            usize,
            &crate::css::style_value::RetainedStyleValueData,
            WrittenValueChecks,
        )>,
        Counter,
    > {
        match winner.source {
            WinnerSource::Rule(rule) => Ok(self
                .program
                .written_winner_declaration(rule, winner.property, winner.important, winner.key.value)
                .map(|(index, value)| (index, value, self.program.written_value_checks(rule, index)))),
            WinnerSource::Element(kind) => {
                let (declared, _) = self.facts.element_declared_properties(node, kind);
                let complete = self
                    .facts
                    .element_declarations_are_complete_but_for_custom_properties(node, kind);
                let written = self.facts.element_written_declared_values(node, kind);
                if !complete || written.len() != declared.len() {
                    return Err(Counter::EngineComputedRecordBailWinnerElement);
                }
                Ok(declared
                    .iter()
                    .rposition(|declared| {
                        declared.property == winner.property
                            && declared.important == winner.important
                            && declared.value == winner.key.value
                    })
                    .map(|index| {
                        (
                            index,
                            &written[index],
                            self.facts.element_written_value_checks(node, kind, index),
                        )
                    }))
            }
            WinnerSource::ExactCascade => Err(Counter::EngineComputedRecordBailWinnerOperator),
        }
    }

    /// Whether a winner's declaration was written with a substitution the engine resolves itself:
    /// var() references of its own, or a longhand pending a shorthand written with them. A value
    /// reading anything else - a custom function, an attribute, a style query - is C++'s, and what
    /// it computes to can move without any winner moving.
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
        declared
            .iter()
            .zip(written)
            .find(|(declared, written)| {
                declared.property < crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
                    && std::ptr::eq(written.pointer(), written_value)
            })
            .map(|(declared, written)| (declared.property, written.clone_retained()))
    }

    /// Whether any winner of a state was written with a substitution, so the record computed
    /// from it reads the node's custom-property environment.
    pub(super) fn state_has_substitutions(&self, node: StyleNodeID, state: CascadeStateID) -> bool {
        self.winner_groups.winners_in_state(state).any(|winner| {
            let Some(winner) = self.winner_groups.resolved_winner(winner) else {
                return false;
            };
            if winner.property < crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID {
                return false;
            }
            let value = match winner.source {
                WinnerSource::Rule(rule) => {
                    self.program
                        .written_winner_value(rule, winner.property, winner.important, winner.key.value)
                }
                WinnerSource::Element(kind) => {
                    let (declared, _) = self.facts.element_declared_properties(node, kind);
                    let written = self.facts.element_written_declared_values(node, kind);
                    declared
                        .iter()
                        .rposition(|declared| {
                            declared.property == winner.property
                                && declared.important == winner.important
                                && declared.value == winner.key.value
                        })
                        .and_then(|index| written.get(index))
                }
                WinnerSource::ExactCascade => None,
            };
            value.is_some_and(|value| {
                matches!(
                    value.data(),
                    crate::css::style_value::StyleValueData::Unresolved { .. }
                        | crate::css::style_value::StyleValueData::PendingSubstitution { .. }
                )
            })
        })
    }

    /// The cascade a winner state describes, as the drive consumes it: every winner's written
    /// value, seeded in cascade order so a logical property pair resolves the way it cascaded.
    /// `None` when a winner is not a plain rule declaration the engine can compute from.
    fn cascaded_store_for_state(
        &mut self,
        node: StyleNodeID,
        state: CascadeStateID,
        pseudo_kind: Option<u8>,
        environment: u64,
        substituted: &mut bool,
        counters: &mut Counters,
    ) -> Option<WinnerStore> {
        use crate::css::property_metadata::property_id as prop;
        crate::css::ffi_stats::bump(crate::css::ffi_stats::FfiOp::WinnerStoreBuilds);
        // Seeded in cascade order, and within one rule in declaration order, since a logical
        // property and its physical associate resolve by order of appearance.
        let mut declarations = Vec::with_capacity(self.winner_groups.winner_count_in_state(state));
        for winner in self.winner_groups.winners_in_state(state) {
            // A revert whose continuation resumes at nothing leaves the property undeclared.
            let Some(winner) = self.winner_groups.resolved_winner(winner) else {
                continue;
            };
            if winner.key.animation_relevance != 0 {
                counters.bump(Counter::EngineComputedRecordBailWinnerAnimated);
                return None;
            }
            // A pseudo-element's cascade keeps the properties its kind supports; its `content`
            // computes in the drive when the value needs no element or counter environment.
            if let Some(kind) = pseudo_kind {
                if !crate::css::property_metadata::pseudo_element_supports_property(kind, winner.property) {
                    continue;
                }
                if winner.property != prop::CONTENT && self.pseudo_winner_needs_cpp(&winner) {
                    counters.bump(Counter::EngineComputedRecordBailProperty);
                    return None;
                }
            }
            // A shorthand written with a substitution is declared beside the longhands it
            // pends; those carry it.
            if winner.property < crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID {
                continue;
            }
            let written = match self.written_winner_value(node, &winner) {
                Ok(written) => written,
                Err(counter) => {
                    counters.bump(counter);
                    None
                }
            };
            let Some((index, value, checks)) = written else {
                counters.bump(Counter::EngineComputedRecordBailWinnerSpelling);
                return None;
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
                        counters.bump(Counter::EngineComputedRecordBailWinnerSpelling);
                        return None;
                    };
                    (location, Some(value))
                }
                // A value with var() references substitutes under the node's environment, as the
                // C++ cascade substitutes it; a value invalid at computed-value time is unset.
                crate::css::style_value::StyleValueData::Unresolved { .. } => {
                    *substituted = true;
                    let value = value.clone_retained();
                    let value = Self::substitute_written_value(
                        &mut self.custom_property_environments,
                        self.document_style_computation_inputs,
                        environment,
                        winner.property,
                        value,
                        counters,
                    )?;
                    (WinnerValue::Substituted(invalid_as_unset(value)), None)
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
                        counters.bump(Counter::EngineComputedRecordBailWinnerSpelling);
                        return None;
                    };
                    let resolved = Self::substitute_written_value(
                        &mut self.custom_property_environments,
                        self.document_style_computation_inputs,
                        environment,
                        shorthand,
                        written,
                        counters,
                    )?;
                    let value = match resolved.data() {
                        crate::css::style_value::StyleValueData::GuaranteedInvalid => unset_value(),
                        _ => expanded_longhand_value(shorthand, winner.property, &resolved).unwrap_or_else(unset_value),
                    };
                    (WinnerValue::Substituted(value), None)
                }
                _ => (location, Some(value.data())),
            };
            let data = match &value {
                WinnerValue::Substituted(value) => value.data(),
                WinnerValue::Written { .. } => borrowed.expect("written declaration is borrowed"),
            };
            // A `url()` resolves against the sheet its rule came from, or the document's base URL
            // for an element's own declaration; a substituted value has lost its sheet.
            let resources_are_known = match &value {
                WinnerValue::Written {
                    source: WinnerSource::Rule(rule),
                    ..
                } => self
                    .rule_source_identity(*rule)
                    .is_some_and(|source| self.document_resource_contexts.for_source(source).is_some()),
                WinnerValue::Written {
                    source: WinnerSource::Element(_),
                    ..
                } => true,
                _ => false,
            };
            let context_free = checks
                .longhand_context_free
                .unwrap_or_else(|| value_computes_without_document_context(data))
                || (resources_are_known && value_computes_without_document_context_but_for_resources(data).is_some());
            if !context_free
                || (pseudo_kind.is_some()
                    && winner.property == prop::CONTENT
                    && !content_value_is_engine_computable(data))
            {
                counters.bump(Counter::EngineComputedRecordBailValue);
                return None;
            }
            if matches!(value, WinnerValue::Substituted(_)) {
                crate::css::ffi_stats::bump(crate::css::ffi_stats::FfiOp::WinnerStoreValueRetains);
            }
            declarations.push((
                winner.priority,
                index,
                WinnerDeclaration::new(winner.property, winner.important, value),
            ));
        }
        declarations.sort_by_key(|(priority, index, ..)| (*priority, *index));
        Some(WinnerStore::new(
            declarations
                .into_iter()
                .map(|(_, _, declaration)| declaration)
                .collect(),
        ))
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
        self.winner_groups
            .semantic_delta_properties(Some(previous_state), state)
            .all(|property| {
                property != crate::css::property_metadata::property_id::CUSTOM
                    && property != crate::css::property_metadata::property_id::DISPLAY
                    && !crate::css::property_metadata::property_is_inherited(property)
            })
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
        let mut scratch = EngineComputabilityScratch::default();
        let publication = self.publish_computed_groups_impl(
            Some(target),
            payloads,
            inherited_group_count,
            custom_property_environment,
            metadata_input,
            computed::PendingRecordOwnership::default(),
            &mut scratch,
            counters,
        );
        let bytes = scratch.capacity_bytes();
        self.memory.reserve_required(MemoryCategory::BatchScratch, bytes);
        drop(scratch);
        self.memory.release(MemoryCategory::BatchScratch, bytes);
        publication
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
            &mut EngineComputabilityScratch::default(),
            counters,
        )
    }

    pub(crate) fn style_record_payloads(&self, style_record: u64) -> Option<&[SharedPayload]> {
        self.computed_group_sets.style_record_payloads(style_record)
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
        scratch: &mut EngineComputabilityScratch,
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
        // A finalized legacy row has already published the root inputs its table produced. Host
        // publication installs the same table later and must not become a second producer.
        let root_font_inputs_were_prepared_from_retained_row =
            target.is_some_and(|target| is_base_record && self.legacy_finalized_longhand_rows.contains_key(&target));
        if is_base_record
            && let Some(target) = target
            && let Some((table_matches, was_host_published)) =
                self.legacy_finalized_longhand_rows.get(&target).map(|row| {
                    (
                        self.style_record_view(publication.style_record_identity.raw())
                            .is_some_and(|record| {
                                unsafe { record.longhand_table.deref() }.publication_equals(row.table())
                            }),
                        row.was_host_published,
                    )
                })
        {
            if table_matches && !was_host_published {
                self.legacy_finalized_longhand_rows
                    .get_mut(&target)
                    .expect("the retained row was just found")
                    .assembled_style_record = publication.style_record_identity.raw();
                self.legacy_finalized_longhand_rows
                    .get_mut(&target)
                    .expect("the retained row was just found")
                    .was_host_published = true;
            } else {
                self.legacy_finalized_longhand_rows.remove(&target);
            }
        }
        if let Some(target) = target
            && !target.is_pseudo()
            && !root_font_inputs_were_prepared_from_retained_row
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
                scratch,
                counters,
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

    pub(crate) fn publish_exact_cascade_state(
        &mut self,
        target: computed::ComputedStyleTarget,
        store: &CascadedPropertyStore,
        inherited_style_groups: u8,
        donor: Option<ExactCascadeDonor>,
        counters: &mut Counters,
    ) -> (bridge::FfiExactCascadePublication, Vec<(u16, SpecifiedWinnerKey)>, bool) {
        let context = self.prepare_exact_cascade_publication(target, donor);
        let had_previous = context.previous.is_some();
        let exact_winners = store
            .winning_declarations()
            .map(|(property, value_pointer, origin, important)| {
                let value = unsafe { self.intern_exact_specified_value(value_pointer) };
                (
                    property,
                    SpecifiedWinnerKey {
                        value,
                        operator: unsafe { Self::cascade_operator_of_style_value(value_pointer) },
                        continuation: cascade::CascadeContinuationID::default(),
                        animation_relevance: match origin {
                            CascadeOrigin::Animation => 1,
                            CascadeOrigin::Transition => 2,
                            _ => 0,
                        },
                        important,
                    },
                )
            })
            .collect::<Vec<_>>();
        let publication = self.publish_exact_cascade_winners_with_context(
            target,
            &exact_winners,
            inherited_style_groups,
            context,
            counters,
        );
        (publication, exact_winners, had_previous)
    }

    pub(crate) fn materialize_retained_cascade_state(
        &mut self,
        target: computed::ComputedStyleTarget,
        store: &mut CascadedPropertyStore,
        blocks: &[FfiCascadeBlock],
    ) -> Vec<FfiSourceSlotAssignment> {
        let Some(answer) = self.current_published_answer(target.node()) else {
            return Vec::new();
        };
        if !answer.cascade_winners_are_complete {
            return Vec::new();
        }
        let key = target.pseudo_element_target().map_or_else(
            || WinnerGroupKey::current(target.node(), self.program.version()),
            |pseudo| WinnerGroupKey::current_pseudo(target.node(), pseudo, self.program.version()),
        );
        let Lookup::Known((_, state)) = self.current_winner_groups().token_for(key) else {
            return Vec::new();
        };

        let mut assignments = Vec::new();
        for winner in self.winner_groups.winners_in_state(state) {
            let Some(winner) = self.winner_groups.resolved_winner(winner) else {
                continue;
            };
            if !Self::retained_store_supports_property(target, winner.property) {
                continue;
            }
            let block = match winner.source {
                WinnerSource::Rule(rule) => {
                    let Some(block) = blocks.iter().find(|block| block.style_engine_rule_id == rule.0 + 1) else {
                        continue;
                    };
                    block
                }
                WinnerSource::Element(ElementDeclarationKind::InlineStyle) => {
                    let Some(block) = blocks.iter().find(|block| block.is_inline_style) else {
                        continue;
                    };
                    block
                }
                WinnerSource::Element(
                    ElementDeclarationKind::PresentationalHint | ElementDeclarationKind::SvgPresentationAttribute,
                ) => {
                    let Some(block) = blocks
                        .iter()
                        .find(|block| block.origin == CascadeOrigin::AuthorPresentationalHint)
                    else {
                        continue;
                    };
                    block
                }
                WinnerSource::ExactCascade => continue,
            };
            let mut source_declarations = block.declarations().filter(|declaration| {
                declaration.property_id == winner.property && declaration.important == winner.important
            });
            let Some(declaration) = source_declarations.next() else {
                continue;
            };
            if source_declarations.next().is_some()
                || !unsafe {
                    self.specified_values
                        .ensure_identity(declaration.data.cast(), winner.key.value, &mut self.memory)
                }
            {
                continue;
            }
            let value = unsafe { &*(declaration.data as *const StyleValueData) };
            if matches!(
                value,
                StyleValueData::Shorthand { .. }
                    | StyleValueData::Unresolved { .. }
                    | StyleValueData::PendingSubstitution { .. }
            ) {
                continue;
            }
            let retained = unsafe {
                RetainedStyleValueData::from_retained_pointer(crate::css::style_value::retain_style_value(
                    declaration.data.cast(),
                ))
            };
            let slot = store.seed_retained_property(
                winner.property,
                retained,
                winner.important,
                declaration.has_style_sheet_context,
            );
            assignments.push(FfiSourceSlotAssignment {
                slot,
                source_id: block.source_id,
            });
        }
        assignments
    }

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
        verify_cascade_winners(self, |verifier| {
            let Some(lower_bound_state) = lower_bound_state else {
                return;
            };
            if !verifier
                .current_published_answer(target.node())
                .is_some_and(|answer| answer.cascade_winners_are_complete)
            {
                return;
            }
            let retained = Arc::clone(
                verifier
                    .retained_match_answer(target.node())
                    .sparse()
                    .expect("a complete published answer retains its exact input"),
            );
            for &(property, exact_key) in exact_winners {
                let Some(maintained_winner) = self.winner_groups.winner_in_state(lower_bound_state, property) else {
                    continue;
                };
                let resolved_winner = self.winner_groups.resolved_winner(maintained_winner);
                if exact_key.animation_relevance != 0
                    || resolved_winner.is_some_and(|winner| {
                        winner.key == exact_key || verifier.winner_is_written_with_substitution(target.node(), &winner)
                    })
                {
                    continue;
                }
                if matches!(
                    maintained_winner.key.operator,
                    CascadeOperator::Revert | CascadeOperator::RevertLayer
                ) && maintained_winner.key.continuation != cascade::CascadeContinuationID::default()
                    && resolved_winner.is_none()
                    && matches!(exact_key.operator, CascadeOperator::Initial | CascadeOperator::Inherit)
                {
                    continue;
                }
                let mut saw_declaration = false;
                let mut declarations_are_unanimous = true;
                let mut inspect = |declared: &DeclaredProperty| {
                    if declared.property == property {
                        saw_declaration = true;
                        declarations_are_unanimous &=
                            declared.value == exact_key.value && declared.operator == exact_key.operator;
                    }
                };
                for matched in retained.iter().filter(|matched| {
                    verifier
                        .programs
                        .get(matched.program)
                        .entries()
                        .get(matched.entry as usize)
                        .is_some_and(|entry| entry.pseudo_element == target.pseudo_element_target())
                }) {
                    verifier
                        .program
                        .declared_properties_of(matched.rule)
                        .iter()
                        .for_each(&mut inspect);
                }
                if !target.is_pseudo() {
                    for kind in ElementDeclarationKind::ALL {
                        verifier
                            .facts
                            .element_declared_properties(target.node(), kind)
                            .0
                            .iter()
                            .for_each(&mut inspect);
                    }
                }
                let unresolved_continuation = matches!(
                    maintained_winner.key.operator,
                    CascadeOperator::Revert | CascadeOperator::RevertLayer
                ) && maintained_winner.key.continuation
                    == cascade::CascadeContinuationID::default();
                let unanimous_declared_mismatch = saw_declaration
                    && declarations_are_unanimous
                    && resolved_winner.is_some_and(|winner| winner.key.operator == CascadeOperator::Declared);
                if !unresolved_continuation && !unanimous_declared_mismatch {
                    continue;
                }
                panic!(
                    "maintained cascade winner {maintained_winner:?}, resolved as {resolved_winner:?}, differs from exact legacy input {exact_key:?} for {target:?}, property {property}; retained input has {} rules",
                    retained.len()
                );
            }
        });
        let state = self.intern_cascade_state(&winners, previous, counters);
        self.winner_groups.settle_memory(&mut self.memory);
        let generation = self.winner_groups.generation();
        let delta = self.winner_groups.semantic_delta(previous, state);
        let unchanged = previous.is_some() && delta.is_empty() && !donor_used;
        let mut computed_property_words = [0u64; crate::css::property_metadata::LONGHAND_WORD_COUNT];
        for property in delta.properties().iter().copied() {
            let Some(index) = property
                .checked_sub(crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID)
                .map(usize::from)
                .filter(|&index| index < crate::css::property_metadata::NUMBER_OF_LONGHAND_PROPERTIES)
            else {
                computed_property_words.fill(u64::MAX);
                break;
            };
            computed_property_words[index / 64] |= 1 << (index % 64);
        }
        // The background longhands form coordinated repeatable lists. A changed layer count in
        // any one of them changes the computed representation of every other list even when its
        // specified winner is unchanged.
        let background_group = 1 << crate::css::computed_value_types::STYLE_GROUP_INDEX_BACKGROUND;
        if delta
            .properties()
            .iter()
            .copied()
            .any(|property| computed_group_output_mask(property).is_some_and(|groups| groups & background_group != 0))
        {
            for property in crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
                ..=crate::css::property_metadata::LAST_LONGHAND_PROPERTY_ID
            {
                if computed_group_output_mask(property).is_some_and(|groups| groups & background_group != 0) {
                    let index = usize::from(property - crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID);
                    computed_property_words[index / 64] |= 1 << (index % 64);
                }
            }
        }
        let current_color_dependency_mask = delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR)
            .then(|| {
                self.computed_group_sets
                    .current_color_dependency_mask(dependency_target)
            });
        let current_color_dependency_properties = delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR)
            .then(|| {
                self.computed_group_sets
                    .current_color_dependency_properties(dependency_target)
            });
        if let Some(Some(dependencies)) = current_color_dependency_properties {
            for (word, dependencies) in computed_property_words.iter_mut().zip(dependencies) {
                *word |= dependencies;
            }
        }
        // caret-color and accent-color bake used values resolved against the element's own color
        // into their group fields, even for their initial `auto`. A color change re-evaluates them
        // whether or not either property is declared anywhere.
        if delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR)
        {
            for property in [
                crate::css::property_metadata::property_id::CARET_COLOR,
                crate::css::property_metadata::property_id::ACCENT_COLOR,
            ] {
                let index = usize::from(property - crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID);
                computed_property_words[index / 64] |= 1 << (index % 64);
            }
        }
        let color_scheme_dependency_mask = delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR_SCHEME)
            .then(|| self.computed_group_sets.color_scheme_dependency_mask(dependency_target));
        let color_scheme_dependency_properties = delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR_SCHEME)
            .then(|| {
                self.computed_group_sets
                    .color_scheme_dependency_properties(dependency_target)
            });
        if let Some(Some(dependencies)) = color_scheme_dependency_properties {
            for (word, dependencies) in computed_property_words.iter_mut().zip(dependencies) {
                *word |= dependencies;
            }
        }
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
        if let Some(Some(dependencies)) = font_dependency_properties {
            for (word, dependencies) in computed_property_words.iter_mut().zip(dependencies) {
                *word |= dependencies;
            }
        }
        let property_closure_is_known = |property: u16| {
            if property == crate::css::property_metadata::property_id::COLOR {
                return current_color_dependency_properties.is_some_and(|properties| properties.is_some());
            }
            if property == crate::css::property_metadata::property_id::COLOR_SCHEME {
                return color_scheme_dependency_properties.is_some_and(|properties| properties.is_some());
            }
            if font_group_mask.is_some() && computed_group_output_mask(property) == font_group_mask {
                return font_dependency_properties.is_some_and(|properties| properties.is_some());
            }
            if !(crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
                ..=crate::css::property_metadata::LAST_LONGHAND_PROPERTY_ID)
                .contains(&property)
            {
                return false;
            }
            crate::css::property_metadata::property_is_in_logical_group(property)
                || crate::css::property_metadata::property_computed_dependents(property).is_some()
        };
        let mut computed_property_closure_is_exact =
            !delta.properties().is_empty() && delta.properties().iter().copied().all(property_closure_is_known);
        if computed_property_closure_is_exact {
            for property in delta.properties().iter().copied() {
                for &dependent in crate::css::property_metadata::property_computed_dependents(property).unwrap_or(&[]) {
                    let index = usize::from(dependent - crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID);
                    computed_property_words[index / 64] |= 1 << (index % 64);
                }
            }
        }
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
        if inherited_property_closure_is_exact {
            for property in crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
                ..=crate::css::property_metadata::LAST_LONGHAND_PROPERTY_ID
            {
                if computed_group_output_mask(property)
                    .is_some_and(|groups| groups & u32::from(inherited_style_groups) != 0)
                {
                    let index = usize::from(property - crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID);
                    computed_property_words[index / 64] |= 1 << (index % 64);
                }
            }
            for dependencies in [
                inherited_current_color_dependency_properties,
                inherited_color_scheme_dependency_properties,
                font_dependency_properties,
            ]
            .into_iter()
            .flatten()
            .flatten()
            {
                for (word, dependencies) in computed_property_words.iter_mut().zip(dependencies) {
                    *word |= dependencies;
                }
            }
            computed_property_closure_is_exact = true;
        }
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
        let selection = StyleComputationSelection {
            computed_property_words,
            computed_property_closure_is_exact,
        };
        if target.is_pseudo() {
            let selections = self
                .pending_pseudo_style_computation_selections
                .entry(target.node())
                .or_default();
            match selections.iter_mut().find(|(kind, _)| *kind == target.pseudo_kind()) {
                Some((_, existing)) => *existing = selection,
                None => selections.push((target.pseudo_kind(), selection)),
            }
        } else {
            self.pending_element_style_computation_selections
                .insert(target.node(), selection);
        }
        bridge::FfiExactCascadePublication {
            unchanged,
            computed_group_mask,
            donor_used,
        }
    }

    pub(crate) fn pending_style_computation_selection(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Option<StyleComputationSelection> {
        let target = computed::ComputedStyleTarget::new(node, pseudo_kind);
        if !target.is_pseudo() {
            return self.pending_element_style_computation_selections.get(&node).copied();
        }
        self.pending_pseudo_style_computation_selections
            .get(&node)?
            .iter()
            .find(|(kind, _)| *kind == pseudo_kind)
            .map(|(_, selection)| *selection)
    }

    pub(crate) fn current_color_dependent_group_mask(&self, node: StyleNodeID, pseudo_kind: u8) -> Option<u32> {
        let target = computed::ComputedStyleTarget::new(node, pseudo_kind);
        let dependencies = self.computed_group_sets.current_color_dependency_mask(target)?;
        let caret_color_group = computed_group_output_mask(crate::css::property_metadata::property_id::CARET_COLOR)?;
        let accent_color_group = computed_group_output_mask(crate::css::property_metadata::property_id::ACCENT_COLOR)?;
        Some(dependencies | caret_color_group | accent_color_group)
    }

    unsafe fn cascade_operator_of_style_value(value: *const StyleValueData) -> CascadeOperator {
        let StyleValueData::Keyword { keyword } = (unsafe { &*value }) else {
            return CascadeOperator::Declared;
        };
        match *keyword {
            crate::css::style_compute::keyword::INHERIT => CascadeOperator::Inherit,
            crate::css::style_compute::keyword::INITIAL => CascadeOperator::Initial,
            crate::css::style_compute::keyword::UNSET => CascadeOperator::Unset,
            crate::css::style_compute::keyword::REVERT => CascadeOperator::Revert,
            crate::css::style_compute::keyword::REVERT_LAYER => CascadeOperator::RevertLayer,
            _ => CascadeOperator::Declared,
        }
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
        verify_cascade_winners(self, |engine| {
            if let Lookup::Known(current) = engine
                .current_winner_groups()
                .token_for(WinnerGroupKey::current(node, engine.program.version()))
            {
                assert_eq!(
                    current, retained,
                    "a reused style must keep the winner state its cascade bound"
                );
            }
        });
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
                    || !self
                        .program
                        .declarations_are_complete_but_for_custom_properties(entry.rule)
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
        self.remove_pending_style_computation_selection(target);
    }

    fn remove_pending_style_computation_selection(&mut self, target: computed::ComputedStyleTarget) {
        if !target.is_pseudo() {
            self.pending_element_style_computation_selections.remove(&target.node());
            return;
        }
        let Some(selections) = self.pending_pseudo_style_computation_selections.get_mut(&target.node()) else {
            return;
        };
        selections.retain(|(kind, _)| *kind != target.pseudo_kind());
        if selections.is_empty() {
            self.pending_pseudo_style_computation_selections.remove(&target.node());
        }
    }

    pub(crate) fn remove_computed_pseudo(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        counters: &mut Counters,
    ) -> Option<computed::FinalStyleRecordID> {
        let target = computed::ComputedStyleTarget::new(node, pseudo_kind);
        self.remove_pending_style_computation_selection(target);
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
    pub(super) fn reclaim_computed_memory_if_needed(&mut self, counters: &mut Counters) {
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
}

/// What a first record reads of the parent's style: its inherited groups, its custom-property
/// environment, its dependency flags, and the display the box-type transformation takes as the
/// parent's. A state that explicitly inherits a non-inherited property reads the parent's whole
/// table, so it keys on the parent's record instead.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ColdRecordParent {
    record: u64,
    inherited_groups: u32,
    environment: u64,
    dependency_flags: u8,
    parent_display: u32,
}

/// What a warm record's cohort is keyed by, and what it answers with: the record, and the style
/// groups it read straight from the parent through an explicit `inherit`.
type RecordCohortKey = (u64, CascadeStateID, u32, RecordDeltaParent, u64, RootFontInputs, i32);
type RecordCohortValue = (computed::FinalStyleRecordID, u32);

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

/// Static checks attached to the original declaration spelling at input preparation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct WrittenValueChecks {
    whole_context_free: bool,
    longhand_context_free: Option<bool>,
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
        Self {
            whole_context_free,
            longhand_context_free,
        }
    }
}

#[derive(Default)]
pub(super) struct EngineComputabilityScratch {
    // NB: Equal winner states can have different per-node written declaration inputs.
    states: HashMap<(StyleNodeID, u64, CascadeStateID, u64, u64), bool>,
}

impl EngineComputabilityScratch {
    fn capacity_bytes(&self) -> u64 {
        capacity::capacity_bytes! {
            shallow [self.states];
            cached [];
            nested [];
            skip [];
        }
    }

    fn remember(&mut self, key: (StyleNodeID, u64, CascadeStateID, u64, u64), admitted: bool) {
        if self.states.len() >= COLD_RECORD_CACHE_LIMIT {
            self.states.clear();
        }
        self.states.insert(key, admitted);
    }
}

#[derive(Default)]
pub(super) struct EngineComputedRecordContinuation {
    pub(super) font_drive: drive::FontDriveScratch,
    // NB: Preserve the root's existing remaining-phase context after preparing consumer inputs.
    root_element_inputs: Option<(StyleNodeID, RootFontInputs)>,
    // NB: A failed preparation already performed the root's unsupported computation.
    root_computation_unsupported: Option<StyleNodeID>,
    root_font_inputs_changed: bool,
    pending_element: Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)>,
    next_pseudo: usize,
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
}

impl EngineComputedRecordContinuation {
    fn capacity_bytes(&self) -> u64 {
        capacity::capacity_bytes! {
            shallow [self.pseudo_deltas, self.substitution_effects];
            cached [self.font_drive.capacity_bytes()];
            nested [];
            skip [];
        }
    }
}

#[derive(Default)]
pub(super) struct EngineComputedRecordScratch {
    /// Whether the transaction moved the document environment, which reaches computed values the
    /// winners do not name.
    pub(super) environment_changed: bool,
    pub(super) continuation: EngineComputedRecordContinuation,
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
    pub(super) prepared_root_font: Option<(StyleNodeID, ParentInputsMoved, drive::FontDriveScratch)>,
    cohorts: HashMap<RecordCohortKey, RecordCohortValue>,
    computability: EngineComputabilityScratch,
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
}

impl std::ops::Deref for EngineComputedRecordScratch {
    type Target = EngineComputedRecordContinuation;

    fn deref(&self) -> &Self::Target {
        &self.continuation
    }
}

impl std::ops::DerefMut for EngineComputedRecordScratch {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.continuation
    }
}

/// What one node tells its flat-tree children, decided where the node settles and read by the
/// children in the same pass: whether the node resolved the record its children inherit from, and
/// the accumulated proof about the chain above it, so a child reads one row instead of walking to
/// the document element for every gate it has to pass.
#[derive(Clone, Copy, Default)]
pub(super) struct DerivedChildInputs {
    /// Whether the node's record settled this flush: what its descendants inherit from is in place.
    pub(super) settled: bool,
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
enum RecordDeltaParent {
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
    pub(super) fn capacity_bytes(&self) -> u64 {
        capacity::capacity_bytes! {
            shallow [self.computability.states, self.cohorts, self.derived_child_inputs, self.cold_cohorts, self.stores,
                self.substituted_states, self.pseudo_cohorts, self.pseudo_stores];
            cached [self.store_capacity_bytes, self.continuation.capacity_bytes(),
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
    /// The element the drive is for, when the record is its own. A pseudo-element's row leaves it
    /// unset: the monospace recascade it would need is the originating element's chain, which this
    /// row does not answer for.
    recascade_node: Option<StyleNodeID>,
    /// The flat-tree parent the element inherits from; the document element has none and
    /// inherits from the initial values.
    parent: Option<StyleNodeID>,
    facts: u32,
}

/// What a retry after an ancestor settles: the element's record, and the pseudo-element records
/// the engine settled beside it, one slot per synthetic kind with a present bit each; a present
/// slot holding zero is a removal.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RetriedEngineRecord {
    pub(crate) style_record: u64,
    pub(crate) pseudo_records_present: u8,
    pub(crate) pseudo_records: [u64; bridge::RETRY_PSEUDO_RECORD_SLOTS],
}

const _: () = assert!(pseudo_kind::SYNTHETIC_COUNT == bridge::RETRY_PSEUDO_RECORD_SLOTS);

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
    inherited_groups: u32,
    parent_display: u32,
    dependency_flags: u8,
    environment: u64,
    kind: u8,
    generation: u64,
    state: Option<CascadeStateID>,
    facts: u32,
    font_environment_generation: u64,
    root_font_inputs: RootFontInputs,
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

    pub(super) fn is_highlight(kind: usize) -> bool {
        kind < SYNTHETIC_COUNT && crate::css::property_metadata::pseudo_element_is_highlight(kind as u8)
    }

    pub(super) fn highlight_mask() -> u64 {
        (0..SYNTHETIC_COUNT)
            .filter(|&kind| is_highlight(kind))
            .fold(0, |mask, kind| mask | 1 << kind)
    }
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

/// Whether a `content` value computes without the element or its counter environment: keywords,
/// and lists of strings and keywords; counters, attributes and images resolve in C++.
fn content_value_is_engine_computable(value: &StyleValueData) -> bool {
    fn plain(value: &StyleValueData) -> bool {
        match value {
            StyleValueData::Keyword { .. } | StyleValueData::String { .. } => true,
            StyleValueData::ValueList { values, .. } => values
                .as_slice()
                .iter()
                .all(|value| value.optional_data().is_none_or(plain)),
            _ => false,
        }
    }
    match value {
        StyleValueData::Keyword { .. } => true,
        StyleValueData::Content { content, alt_text } => {
            content.optional_data().is_none_or(plain) && alt_text.optional_data().is_none_or(plain)
        }
        _ => false,
    }
}

/// The value a shorthand value carries for one of its longhands, through nested shorthands.
/// The `unset` keyword, which a declaration invalid at computed-value time computes as.
fn unset_value() -> crate::css::style_value::RetainedStyleValueData {
    crate::css::style_value::RetainedStyleValueData::from_owned(crate::css::style_value::StyleValueData::Keyword {
        keyword: crate::css::style_compute::keyword::UNSET,
    })
}

fn invalid_as_unset(
    value: crate::css::style_value::RetainedStyleValueData,
) -> crate::css::style_value::RetainedStyleValueData {
    match value.data() {
        crate::css::style_value::StyleValueData::GuaranteedInvalid => unset_value(),
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

/// Whether a longhand computes in the drive's remaining phase: after the font, line-height and
/// color-scheme stages, whose outputs the engine does not derive itself yet.
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

/// The counter-style names no @counter-style rule overrides: decimal, disc, square, circle,
/// disclosure-open and disclosure-closed.
fn counter_style_name_is_non_overridable(name: &[u16]) -> bool {
    [
        "decimal",
        "disc",
        "square",
        "circle",
        "disclosure-open",
        "disclosure-closed",
    ]
    .iter()
    .any(|candidate| {
        candidate.len() == name.len()
            && candidate
                .bytes()
                .zip(name)
                .all(|(expected, &unit)| unit < 128 && (unit as u8).eq_ignore_ascii_case(&expected))
    })
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

fn property_starts_animation_or_counter_environment(property: u16) -> bool {
    use crate::css::property_metadata::{
        FIRST_LONGHAND_PROPERTY_ID, LAST_LONGHAND_PROPERTY_ID, property_id as prop, property_style_group_index,
    };
    if !(FIRST_LONGHAND_PROPERTY_ID..=LAST_LONGHAND_PROPERTY_ID).contains(&property) {
        return true;
    }
    // A view transition name is a plain computed value; it starts nothing. An anchor name is one
    // the host registers from whichever record it installs. A named timeline is a plain computed
    // value too: what finds it is the animation that names it in `animation-timeline`, and that
    // animation reads it from whichever record the element holds when it starts.
    matches!(property, prop::CONTENT | prop::LIST_STYLE_TYPE)
        || (!matches!(
            property,
            prop::VIEW_TRANSITION_NAME
                | prop::SCROLL_TIMELINE_NAME
                | prop::SCROLL_TIMELINE_AXIS
                | prop::TIMELINE_SCOPE
                | prop::VIEW_TIMELINE_NAME
                | prop::VIEW_TIMELINE_AXIS
                | prop::VIEW_TIMELINE_INSET
        ) && property_style_group_index(property)
            .is_some_and(|group| usize::from(group) == crate::css::table_group_builder::group_index::ANIMATION))
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
    fn retained_inheritance_parent_uses_tree_and_element_backed_pseudo_records() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut raw_nodes = [0; 8];
        engine.allocate_style_nodes(&mut raw_nodes);
        let [
            parent,
            child,
            host,
            light_child,
            shadow_root,
            wrapper,
            represented,
            slot,
        ] = raw_nodes.map(|node| StyleNodeID::from_raw(node).unwrap());
        engine.tree.set_parent(child, Some(parent));
        engine.tree.set_parent(light_child, Some(host));
        engine.tree.set_parent(wrapper, Some(shadow_root));
        engine.tree.set_parent(represented, Some(wrapper));
        engine
            .state
            .retained
            .tree
            .set_shadow_root(host, shadow_root, &mut engine.state.retained.memory);
        engine
            .computed_group_sets
            .set_associated_pseudo_kind(represented, bridge::FIRST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND + 1);
        let publish = |engine: &mut StyleEngine, node| {
            engine
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
                .style_record_identity
        };
        let parent_record = publish(&mut engine, parent);
        let child_record = publish(&mut engine, child);
        let host_record = publish(&mut engine, host);
        let wrapper_record = publish(&mut engine, wrapper);
        let slot_record = publish(&mut engine, slot);

        assert_eq!(
            engine.retained_inheritance_parent_style_record(child, u8::MAX),
            Some(parent_record)
        );
        assert_eq!(
            engine.retained_inheritance_ancestor_style_records(child, u8::MAX),
            [parent_record.raw()]
        );
        assert_eq!(
            engine.retained_inheritance_parent_style_record(child, 0),
            Some(child_record)
        );
        assert_eq!(
            engine.retained_inheritance_parent_style_record(host, bridge::FIRST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND,),
            Some(wrapper_record)
        );
        assert_eq!(
            engine.retained_inheritance_ancestor_style_records(
                host,
                bridge::FIRST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND,
            ),
            [wrapper_record.raw()]
        );
        assert_eq!(engine.tree.flat_tree_parent(light_child), None);
        assert_eq!(
            engine.retained_inheritance_parent_style_record(light_child, u8::MAX),
            Some(host_record)
        );
        let retained = &mut engine.state.retained;
        retained
            .tree
            .set_assigned_slot(light_child, Some(slot), &mut retained.memory);
        assert_eq!(
            engine.retained_inheritance_parent_style_record(light_child, u8::MAX),
            Some(slot_record)
        );
        assert_eq!(
            engine.retained_inheritance_ancestor_style_records(light_child, u8::MAX),
            [slot_record.raw()]
        );
    }

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
    fn computability_scratch_keeps_element_declaration_inputs_separate() {
        use crate::css::style_value::RetainedStyleValueData;

        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut raw_nodes = [0; 2];
        engine.allocate_style_nodes(&mut raw_nodes);
        let [first, second] = raw_nodes.map(|node| StyleNodeID::from_raw(node).unwrap());
        let declaration = DeclaredProperty {
            property: crate::css::property_metadata::property_id::OPACITY,
            important: false,
            operator: CascadeOperator::Declared,
            value: SpecifiedValueID(1),
        };
        let kind = ElementDeclarationKind::InlineStyle;
        engine.facts.set_element_declared_properties(
            first,
            kind,
            vec![declaration],
            vec![RetainedStyleValueData::from_owned(StyleValueData::Number {
                value: 0.5,
            })],
            true,
        );
        engine
            .facts
            .set_element_declared_properties(second, kind, vec![declaration], Vec::new(), true);
        let winner = PropertyWinner {
            property: declaration.property,
            important: false,
            key: SpecifiedWinnerKey {
                value: declaration.value,
                operator: declaration.operator,
                continuation: cascade::CascadeContinuationID::default(),
                animation_relevance: 0,
                important: false,
            },
            priority: CascadePriority::exact_output_placeholder(),
            source: WinnerSource::Element(kind),
        };
        let state = engine.winner_groups.intern_sorted(&[winner], None);
        let cascade_state = (0, state);
        for order in [[first, second], [second, first]] {
            let mut scratch = EngineComputabilityScratch::default();
            for node in order {
                assert_eq!(
                    engine
                        .state
                        .state_is_engine_computable(node, cascade_state, &mut scratch, &mut engine.counters),
                    node == first,
                );
            }
        }
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
        let mut scratch = EngineComputedRecordScratch::default();
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

        engine
            .state
            .abandon_engine_computed_record(third, &mut scratch, &mut engine.counters);
        assert_eq!(engine.computed_group_sets.assigned_style_record(third), None);
        assert_eq!(engine.computed_group_sets.pseudo_style_record(third, 0), None);
        assert!(engine.engine_computed_records_pending.is_empty());

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
}

impl StyleEngineState {
    /// Keep the style record already assigned to a target whose recomputation its input record
    /// answered. Returns nothing when the target has no assignment or recording is active, so the
    /// caller publishes the style in full instead.
    pub(crate) fn reaffirm_style_record(
        &mut self,
        target: computed::ComputedStyleTarget,
        counters: &mut Counters,
    ) -> Option<computed::FinalStyleRecordID> {
        if self.recording_id().is_some() {
            return None;
        }
        let style_record = self.computed_group_sets.assigned_final_style_record(target)?;
        if let Some(current_cascade_state) = self.computed_group_sets.take_pending_cascade_state(target) {
            self.bind_published_cascade_state(target, current_cascade_state, false, counters);
            let view = self
                .computed_group_sets
                .style_record_view(style_record.raw())
                .expect("an assigned style record must be live");
            let is_base_record = view.animation_overlay_identity == 0;
            let pseudo_styles = view.pseudo_element_styles;
            if let Some(custom_property_environment) = self
                .computed_group_sets
                .custom_property_environment_identity(target.node())
            {
                self.remember_cold_record_candidate(
                    target,
                    current_cascade_state,
                    custom_property_environment,
                    pseudo_styles,
                    Some(style_record),
                    style_record,
                    is_base_record,
                    &mut EngineComputabilityScratch::default(),
                    counters,
                );
            }
        }
        counters.bump(Counter::StyleRecordsReaffirmed);
        Some(style_record)
    }

    pub(crate) fn end_style_record_view_epoch(&mut self, counters: &mut Counters) {
        self.retained.computed_group_sets.end_style_record_view_epoch();
        self.reclaim_computed_memory_if_needed(counters);
    }
}

impl StyleEngineState {
    pub(super) fn refill_font_requests(
        &mut self,
        requests: Vec<(Option<StyleNodeID>, font_resolution::FontRequest)>,
        counters: &mut Counters,
    ) {
        self.refill_font_requests_for_service(requests, font_resolution::FontService::ParkedBatch, counters);
    }

    fn refill_font_requests_for_service(
        &mut self,
        requests: Vec<(Option<StyleNodeID>, font_resolution::FontRequest)>,
        service: font_resolution::FontService,
        counters: &mut Counters,
    ) {
        if requests.is_empty() {
            return;
        }
        // NB: Use resident selector-tree depths for this diagnostic. They are not flat-tree
        //     dependency-span proofs and must not buy ancestor traversals just for counting.
        counters.set(
            Counter::FontRefillBlockedDepth,
            requests
                .iter()
                .fold(counters.get(Counter::FontRefillBlockedDepth), |depth, (node, _)| {
                    node.map_or(depth, |node| depth.max(u64::from(self.tree.depth(node)) + 1))
                }),
        );
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
        let request_count = resolver.refill(
            memo,
            snapshot.as_ref(),
            resolutions,
            requests.into_iter().map(|(_, request)| request).collect(),
            service,
        );
        if request_count != 0 {
            counters.bump(Counter::FontRefillRounds);
            counters.add(Counter::FontResolutionRequests, request_count as u64);
        }
    }
}

impl StyleEngineState {
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
        let Some(inputs) = self.document_style_computation_inputs else {
            counters.bump(Counter::RootFontInputsUnprovenFallbacks);
            return;
        };
        let assigned_style_record = self.computed_group_sets.assigned_style_record(node);
        if (parent_inputs_moved.inherited_style && !self.engine_marker_font_supported(node, counters))
            || !self.engine_pseudo_inputs_available(node, assigned_style_record, counters)
        {
            counters.bump(Counter::RootFontInputsUnprovenFallbacks);
            return;
        }
        scratch.root_element_inputs = Some((node, RootFontInputs::from_document(&inputs)));
        self.engine_computed_element_record_delta(
            node,
            cascade_winners_are_complete,
            exact_flipped_rules,
            parent_inputs_moved,
            scratch,
            FontDriveGoal::RootInputs,
            counters,
        );
        if let Some(request) = scratch.font_drive.request.take() {
            self.root_font_request = Some(request.for_generation(inputs.font_environment_generation));
            // This update computed a new root request after the begin boundary. Complete that
            // exceptional miss through the shared between-pass service before consumers run.
            self.refill_font_requests(vec![(Some(node), request)], counters);
            self.engine_computed_element_record_delta(
                node,
                cascade_winners_are_complete,
                exact_flipped_rules,
                parent_inputs_moved,
                scratch,
                FontDriveGoal::RootInputs,
                counters,
            );
        }
        let prepared = scratch.font_drive.root_inputs.take();
        if let Some(root_inputs) = prepared {
            scratch.root_font_inputs_changed = RootFontInputs::from_document(&inputs) != root_inputs;
            root_inputs.apply_to(self.document_style_computation_inputs.as_mut().unwrap());
            counters.bump(Counter::RootFontInputsPrepared);
        } else {
            // NB: Preserve the current host root-metric route. Unproven font inputs do not
            //     turn every descendant into a host-boundary retry.
            counters.bump(Counter::RootFontInputsUnprovenFallbacks);
        }
        if prepared.is_none() && !scratch.font_drive.is_pending() && !scratch.font_drive.root_inputs_unproven {
            scratch.root_computation_unsupported = Some(node);
        }
        if scratch.font_drive.is_pending() {
            scratch.prepared_root_font = Some((node, parent_inputs_moved, std::mem::take(&mut scratch.font_drive)));
        }
        self.apply_substitution_effects(scratch);
    }

    /// Retry a record after C++ has installed earlier records in the same preorder batch. A record
    /// rejected while the batch was planned may become computable once its inheritance parent is
    /// authoritative.
    /// Record one way the host entered the engine for one element, under the reason the engine
    /// sent it there. The engine knows the reason; the host knows when the entry happens, so the
    /// two halves meet here. `row_kinds` is what the host's own row census already carries.
    pub(crate) fn note_host_entry(&mut self, node: StyleNodeID, kind: u8, row_kinds: u8) {
        if !seal::is_reporting() {
            return;
        }
        let kind = match kind {
            1 => seal::HostEntryKind::Retry,
            2 => seal::HostEntryKind::Sampled,
            _ => seal::HostEntryKind::Row,
        };
        let recorded = self.retained.host_entry_causes.get(&node).copied();
        let (cause, cold) = recorded.unwrap_or_else(|| {
            // The record loop was never offered this element, so no gate declined it. Name the
            // way in instead: this population has never been ranked beside the declines.
            let cause = if row_kinds & (1 << 1) != 0 {
                "NotOfferedPseudoElement"
            } else if row_kinds & (1 << 3) != 0 {
                "NotOfferedHighlightParent"
            } else if row_kinds & (1 << 4) != 0 {
                "NotOfferedLonghandDriveOnly"
            } else if row_kinds & (1 << 0) != 0 {
                "InBatchWithoutDecline"
            } else {
                "NotOfferedOutOfBatch"
            };
            (
                cause,
                self.retained.computed_group_sets.assigned_style_record(node).is_none(),
            )
        });
        seal::note_host_entry(cause, kind, cold);
    }

    /// Settle every armed row the ancestor the host has just applied unblocks, in flat-tree
    /// order, as one crossing. The host asks about one node; the rows beside and below it wait on
    /// the same ancestor and would each have asked for themselves. A row is taken only when its
    /// flat-tree parent is the ancestor this call is anchored to, or a row this same call already
    /// settled, so nothing here reads a record the host has yet to install.
    pub(crate) fn retry_engine_records_after_ancestor(&mut self, node: StyleNodeID, counters: &mut Counters) {
        self.host.retried_record_rows.clear();
        self.note_host_entry(node, 1, 0);
        let anchor = self.tree.flat_tree_parent(node);
        let armed = std::mem::take(&mut self.host.armed_retry_nodes);
        let mut settled: HashSet<StyleNodeID> = HashSet::default();
        let mut still_armed = Vec::with_capacity(armed.len());
        let mut reached = false;
        for candidate in armed {
            let unblocked = if candidate == node {
                reached = true;
                true
            } else if !reached {
                false
            } else {
                self.tree
                    .flat_tree_parent(candidate)
                    .is_some_and(|parent| Some(parent) == anchor || settled.contains(&parent))
            };
            if !unblocked {
                still_armed.push(candidate);
                continue;
            }
            let retried = self.retry_engine_record_after_ancestor(candidate, counters);
            if retried.style_record == 0 {
                continue;
            }
            settled.insert(candidate);
            let uses_substitution = self.nodes_with_substituted_records.contains(&candidate);
            self.host.retried_record_rows.push(bridge::FfiRetriedRecordRow {
                style_node: candidate.raw(),
                record: bridge::FfiEngineComputedRecord {
                    style_record: retried.style_record,
                    uses_substitution,
                    pseudo_records_present: retried.pseudo_records_present,
                    pseudo_records: retried.pseudo_records,
                },
            });
        }
        self.host.armed_retry_nodes = still_armed;
    }

    pub(crate) fn retry_engine_record_after_ancestor(
        &mut self,
        node: StyleNodeID,
        counters: &mut Counters,
    ) -> RetriedEngineRecord {
        if let Some(inputs) = self.retained.document_style_computation_inputs
            && let Some(resolver) = &mut self.retained.font_resolution
        {
            resolver.prepare(inputs.font_environment_generation);
        }
        counters.bump(Counter::RetryAfterAncestorCalls);
        let started_at = std::time::Instant::now();
        let mut scratch = EngineComputedRecordScratch::default();
        let mut suspended_memory = MemoryLease::new(MemoryCategory::BatchScratch);
        let style_record =
            self.retry_engine_record_after_ancestor_loop(node, &mut scratch, &mut suspended_memory, counters);
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

    fn retry_engine_record_after_ancestor_loop(
        &mut self,
        node: StyleNodeID,
        scratch: &mut EngineComputedRecordScratch,
        suspended_memory: &mut MemoryLease,
        counters: &mut Counters,
    ) -> u64 {
        loop {
            let record = self.retry_engine_record_after_ancestor_step(node, scratch, counters);
            let Some(request) = scratch.font_drive.request.take() else {
                if record != 0 {
                    counters.bump(Counter::RetryAfterAncestorSettled);
                }
                return record;
            };
            suspended_memory.resize_required_to(&mut self.memory, scratch.font_drive.capacity_bytes());
            // C++ installs the earlier ancestor before making this retry, so requests from
            // different retries cannot be known together. Use the shared batch service even
            // though this dependency boundary limits the batch to one request.
            self.refill_font_requests(vec![(Some(node), request)], counters);
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
