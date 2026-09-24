/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

impl RetainedState {
    /// A pseudo row can predate the current answer even when the element row is current. Rebuild
    /// its winners from that answer before deciding whether the engine can settle its record.
    fn refresh_stale_pseudo_winners(
        &mut self,
        node: StyleNodeID,
        new_element_record: computed::FinalStyleRecordID,
        counters: &mut Counters,
    ) {
        let Some(mask) = self.pseudo_style_mask(node) else {
            return;
        };
        let new_is_list_item = self
            .computed_group_sets
            .style_record_view(new_element_record.raw())
            .and_then(|view| unsafe { view.longhand_table.as_ref() })
            .is_some_and(|table| table.display_is_list_item());
        let marker_is_live = new_is_list_item
            || self
                .computed_group_sets
                .pseudo_style_record(node, pseudo_kind::MARKER)
                .is_some()
            || [pseudo_kind::BEFORE, pseudo_kind::AFTER, pseudo_kind::BACKDROP]
                .into_iter()
                .filter_map(|kind| self.computed_group_sets.pseudo_style_record(node, kind))
                .filter_map(|record| self.computed_group_sets.style_record_view(record.raw()))
                .filter_map(|view| unsafe { view.longhand_table.as_ref() })
                .any(|table| table.display_is_list_item());
        let stale = self
            .current_winner_groups()
            .pseudo_states(node)
            .any(|(pseudo, version, _, priority_current)| {
                let kind = usize::from(pseudo.kind.0);
                kind < pseudo_kind::SYNTHETIC_COUNT
                    && mask & (1_u64 << kind) != 0
                    && (kind != usize::from(pseudo_kind::BACKDROP) || self.top_layer_elements.contains(&node))
                    && (kind != usize::from(pseudo_kind::MARKER) || marker_is_live)
                    && (version != self.program.version() || !priority_current)
            });
        if stale {
            if self.current_winner_groups().row_stamp(node) == Some(self.flush_stamp) {
                self.republish_pseudo_winners_from_retained_answer(node, counters);
            } else {
                self.republish_winners_from_answer(node, counters);
            }
        }
    }

    pub(crate) fn drop_demand_pseudo_records(&mut self, node: StyleNodeID) {
        let records: Vec<_> = self
            .demand_pseudo_records
            .extract_if(|(owner, _), _| *owner == node)
            .map(|(_, record)| record)
            .collect();
        for record in records {
            self.computed_group_sets.unpin_style_record(record.raw());
        }
    }

    /// Check pseudo winner availability before deriving an originating record that would have
    /// to be discarded. Marker generation additionally depends on the newly computed display
    /// and is checked when settling the pseudo records.
    pub(super) fn engine_pseudo_inputs_available(
        &mut self,
        node: StyleNodeID,
        record: Option<computed::FinalStyleRecordID>,
        counters: &mut Counters,
    ) -> bool {
        use pseudo_kind::{AFTER, BEFORE, FIRST_LETTER, MARKER, SELECTION};

        // A marker is generated for a list item only: the stale marker row of an element that is
        // no list item decides nothing. Settling the records checks it again against the
        // element's new record.
        let record_is_list_item = record
            .and_then(|record| self.computed_group_sets.style_record_view(record.raw()))
            .and_then(|view| unsafe { view.longhand_table.as_ref() })
            .is_some_and(|table| table.display_is_list_item());
        // What the element's display winner says: `None` when the state or the value cannot say.
        let display_winner_is_list_item = match self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        {
            Lookup::Known((_, state)) => match self
                .winner_groups
                .winner_in_state(state, crate::css::property_metadata::property_id::DISPLAY)
                .and_then(|winner| self.winner_groups.resolved_winner(winner))
            {
                None => Some(false),
                Some(winner) => match self.specified_values.value(winner.key.value) {
                    Lookup::Known(StyleValueData::Display { raw }) => {
                        Some(crate::css::display::FfiDisplay::from_raw(*raw).is_list_item())
                    }
                    _ => None,
                },
            },
            _ => None,
        };
        let marker_may_generate = record_is_list_item || display_winner_is_list_item != Some(false);
        let mut available = 0_u64;
        for (pseudo, version, _, priority_current) in self.current_winner_groups().pseudo_states(node) {
            if self.deferred_pseudo_element == Some(pseudo.kind) {
                continue;
            }
            let kind = usize::from(pseudo.kind.0);
            if kind >= pseudo_kind::SYNTHETIC_COUNT
                || (pseudo_kind::is_highlight(kind) && kind != usize::from(SELECTION))
            {
                continue;
            }
            if version != self.program.version() || !priority_current {
                if kind == usize::from(MARKER) && !marker_may_generate {
                    continue;
                }
                // A row the node's current answer has no rules for is one left from rules that
                // no longer match: the pseudo-element it styled is not generated.
                if self.pseudo_style_mask(node).is_some_and(|mask| mask & (1 << kind) == 0) {
                    continue;
                }
                // Settlement refreshes this row from the node's retained match answer after
                // its element record is decided. Refreshing now would also replace the element
                // winner row before its normal computed-output comparison.
                if self
                    .current_answer_identity(node)
                    .and_then(|identity| self.match_answers.answer(identity))
                    .is_some()
                {
                    available |= 1 << kind;
                    continue;
                }
                counters.bump(Counter::EngineComputedRecordBailPseudoStale);
                return false;
            }
            available |= 1 << kind;
        }
        let Some(mut required) = self.pseudo_style_mask(node) else {
            counters.bump(Counter::EngineComputedRecordBailPseudoMask);
            return false;
        };
        if let Some(deferred) = self.deferred_pseudo_element {
            required &= !(1_u64 << deferred.0);
        }
        required &= !pseudo_kind::highlight_mask() | (1 << SELECTION);
        if required & !available == 0 {
            return true;
        }
        let explicit_kinds = (1 << BEFORE) | (1 << AFTER) | (1 << FIRST_LETTER) | (1 << SELECTION);
        if required & explicit_kinds & !available != 0 {
            counters.bump(Counter::EngineComputedRecordBailPseudoRow);
            return false;
        }
        true
    }

    /// Settle the synthetic pseudo-elements of an element the engine derived a record for, the
    /// way the C++ computation refreshes them after the element's own: each kind the element has
    /// rules for, and the marker a list item generates, is driven against the element's new
    /// record; one that generates no box any more is removed; one whose cascade state did not
    /// move keeps its record. `None` is a pseudo-element the engine cannot settle.
    pub(super) fn engine_pseudo_records(
        &mut self,
        node: StyleNodeID,
        old_element_record: Option<computed::FinalStyleRecordID>,
        new_element_record: computed::FinalStyleRecordID,
        generation: u64,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Drive<()> {
        self.drop_demand_pseudo_records(node);
        self.settle_engine_pseudo_records(
            node,
            old_element_record,
            None,
            new_element_record,
            generation,
            scratch,
            counters,
            None,
            false,
            None,
            false,
        )
    }

    /// Whether the host applies a CSS animation plan the engine settles for a pseudo-element: its
    /// state names animations, or the pseudo-element holds CSS animations the state no longer
    /// names, which an empty plan cancels.
    fn pseudo_owes_css_animation_plan(&self, node: StyleNodeID, kind: u8, state: Option<CascadeStateID>) -> bool {
        state.is_some_and(|state| !self.state_has_no_animation_name(state))
            || !self.element_css_defined_animations(node, kind + 1).is_empty()
    }

    /// `old_is_list_item` says whether the element was a list item when the old record it no
    /// longer holds is unknown: C++ computed the new one over it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn settle_engine_pseudo_records(
        &mut self,
        node: StyleNodeID,
        old_element_record: Option<computed::FinalStyleRecordID>,
        old_is_list_item: Option<bool>,
        new_element_record: computed::FinalStyleRecordID,
        generation: u64,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
        selected_kind: Option<u8>,
        cssom_read: bool,
        highlight_parent: Option<computed::FinalStyleRecordID>,
        observe_without_box: bool,
    ) -> Drive<()> {
        use pseudo_kind::{AFTER, BACKDROP, BEFORE, FIRST_LETTER, MARKER, SELECTION};

        self.refresh_stale_pseudo_winners(node, new_element_record, counters);

        let Some(mut inputs) = self.document_style_computation_inputs else {
            counters.bump(Counter::EngineComputedRecordBailNoEnvironment);
            return Err(Unanswered::Refused);
        };
        // NB: Root pseudos use the originating record's current font, independently of
        //     the document context used for the root's own remaining properties.
        if self.computed_group_sets.adjustment_facts(node) & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT != 0 {
            self.root_font_inputs_from_record(new_element_record)
                .or_refused()?
                .apply_to(&mut inputs);
        }
        let program_version = self.program.version();
        let in_top_layer = self.top_layer_elements.contains(&node);
        let mut states: [Option<CascadeStateID>; 20] = [None; 20];
        let mut marker_row_is_stale = false;
        for (pseudo, version, state, priority_current) in self.current_winner_groups().pseudo_states(node) {
            if selected_kind.is_none() && self.deferred_pseudo_element == Some(pseudo.kind) {
                continue;
            }
            let Ok(kind) = u8::try_from(pseudo.kind.0) else {
                continue;
            };
            if usize::from(kind) >= states.len() {
                continue;
            }
            if selected_kind.is_some_and(|selected| selected != kind) {
                continue;
            }
            if version != program_version || !priority_current {
                // The backdrop of a node outside the top layer cannot generate a box. Its
                // retained winner may be stale without changing this transaction's answer.
                if kind == BACKDROP && selected_kind.is_none() && !in_top_layer {
                    states[usize::from(kind)] = Some(state);
                    continue;
                }
                // A row the node's current answer has no rules for is one left from rules that
                // no longer match: the pseudo-element it styled is not generated.
                if self.pseudo_style_mask(node).is_some_and(|mask| mask & (1 << kind) == 0) {
                    continue;
                }
                // Whether a marker is generated is known once the element's display is.
                if kind == MARKER {
                    marker_row_is_stale = true;
                    continue;
                }
                counters.bump(Counter::EngineComputedRecordBailPseudoStale);
                return Err(Unanswered::Refused);
            }
            states[usize::from(kind)] = Some(state);
        }
        let display_is_list_item = |engine: &Self, record: computed::FinalStyleRecordID| -> Option<bool> {
            let view = engine.computed_group_sets.style_record_view(record.raw())?;
            let table = unsafe { view.longhand_table.as_ref() }?;
            Some(table.display_is_list_item())
        };
        let Some(new_is_list_item) = display_is_list_item(self, new_element_record) else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return Err(Unanswered::Refused);
        };
        let Some(new_view_dependency_flags) = self
            .computed_group_sets
            .style_record_view(new_element_record.raw())
            .map(|view| view.dependency_flags)
        else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return Err(Unanswered::Refused);
        };
        let old_is_list_item = match (old_is_list_item, old_element_record) {
            (Some(old_is_list_item), _) => old_is_list_item,
            (None, Some(record)) => {
                let Some(list_item) = display_is_list_item(self, record) else {
                    counters.bump(Counter::EngineComputedRecordBailRecord);
                    return Err(Unanswered::Refused);
                };
                list_item
            }
            (None, None) => false,
        };
        if marker_row_is_stale && (new_is_list_item || old_is_list_item) {
            counters.bump(Counter::EngineComputedRecordBailPseudoStale);
            return Err(Unanswered::Refused);
        }
        // What a pseudo-element inherits from its element: an element record that kept its
        // inherited groups left them alone.
        let inherited_inputs_unchanged = match old_element_record {
            Some(old) if old == new_element_record => true,
            Some(old) => {
                let old_identity = self.computed_group_sets.style_record_inherited_groups_identity(old);
                let new_identity = self
                    .computed_group_sets
                    .style_record_inherited_groups_identity(new_element_record);
                old_identity.is_some() && old_identity == new_identity
            }
            None => false,
        };
        let facts = self.computed_group_sets.adjustment_facts(node) & PSEUDO_ELEMENT_ADJUSTMENT_FACTS;
        // A pseudo-element resolves a font cascade of its own, so its originating inputs moved
        // when the element's font environment did, exactly as when the root's font inputs did.
        let originating_inputs_unchanged = !cssom_read
            && inherited_inputs_unchanged
            && !scratch.root_font_inputs_changed
            && !scratch.viewport_moved
            && !scratch.font_environment_moved
            && !scratch.document_environment_moved
            && old_element_record.is_some_and(|old| {
                let Some(old_view) = self.computed_group_sets.style_record_view(old.raw()) else {
                    return false;
                };
                let Some(new_view) = self.computed_group_sets.style_record_view(new_element_record.raw()) else {
                    return false;
                };
                let (Some(old_table), Some(new_table)) = (unsafe { old_view.longhand_table.as_ref() }, unsafe {
                    new_view.longhand_table.as_ref()
                }) else {
                    return false;
                };
                let old_display = crate::css::style_compute::effective_display(old_table, None);
                let new_display = crate::css::style_compute::effective_display(new_table, None);
                old_view.dependency_flags == new_view.dependency_flags
                    && old_display == new_display
                    && !new_display.is_contents()
                    && self
                        .computed_group_sets
                        .style_record_custom_property_environment(old.raw())
                        == self
                            .computed_group_sets
                            .style_record_custom_property_environment(new_element_record.raw())
            });
        let Some(element_environment) = self
            .computed_group_sets
            .style_record_custom_property_environment(new_element_record.raw())
            .or_else(|| {
                self.computed_group_sets
                    .animation_overlay_base_custom_property_environment(new_element_record.raw())
            })
        else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return Err(Unanswered::Refused);
        };
        // The kinds the node's match answer has rules for: a winner row is published for each
        // the engine cascaded itself, and a kind with rules but no row is not decided.
        let Some(kinds_with_rules) = self.pseudo_style_mask(node) else {
            counters.bump(Counter::EngineComputedRecordBailPseudoMask);
            return Err(Unanswered::Refused);
        };
        // A marker's named counter style can move without changing any inherited group or
        // winner. Both retained and shared pseudo records must name the current registry.
        let mut pseudo_uses_substitution = scratch.pseudo_uses_substitution;
        for (pseudo_index, kind) in [BEFORE, AFTER, FIRST_LETTER, SELECTION, BACKDROP, MARKER]
            .into_iter()
            .chain(
                selected_kind.filter(|kind| ![BEFORE, AFTER, FIRST_LETTER, SELECTION, MARKER, BACKDROP].contains(kind)),
            )
            .enumerate()
            .skip(scratch.next_pseudo)
        {
            scratch.next_pseudo = pseudo_index + 1;
            if selected_kind.is_some_and(|selected| selected != kind) {
                continue;
            }
            if selected_kind.is_none() && self.deferred_pseudo_element == Some(tree::PseudoElementKind(u16::from(kind)))
            {
                continue;
            }
            // The backdrop of a node outside the top layer generates no box, whatever its rules.
            if selected_kind.is_none() && kind == BACKDROP && !in_top_layer {
                self.computed_group_sets.remove_pseudo(node, kind);
                continue;
            }
            let target = computed::ComputedStyleTarget::new(node, kind);
            let old = self.computed_group_sets.pseudo_style_record(node, kind);
            // The host runs the pseudo's transition step when it installs the driven record, with the
            // old record as the before-change style, including any composition it holds, and it
            // samples the pseudo's effects after installation.
            let pin_old_composition = match old {
                Some(old) => {
                    let Some(view) = self.computed_group_sets.style_record_view(old.raw()) else {
                        counters.bump(Counter::EngineComputedRecordBailRecord);
                        return Err(Unanswered::Refused);
                    };
                    !view.animated_overlay.is_null()
                }
                None => false,
            };
            // A marker is generated for a list item (and refreshed once more for an element that
            // stops being one), whatever rules match ::marker; other kinds are generated by their
            // rules, which the answer names: a winner row outlives the last rule as an empty
            // state, and a rule without declarations is an empty state that generates.
            // A list-item generated box also needs the originating element's marker
            // record, even when the originating element itself is not a list item.
            let pseudo_is_list_item = kind == MARKER
                && [BEFORE, AFTER, BACKDROP].into_iter().any(|pseudo| {
                    self.computed_group_sets
                        .pseudo_style_record(node, pseudo)
                        .and_then(|record| display_is_list_item(self, record))
                        == Some(true)
                });
            let implicit = kind == MARKER && (new_is_list_item || old_is_list_item || pseudo_is_list_item);
            if kind == MARKER && !implicit && !cssom_read {
                continue;
            }
            let has_rules = kinds_with_rules & (1 << kind) != 0
                || (cssom_read && selected_kind == Some(kind) && states[usize::from(kind)].is_some());
            let highlight_parent_record = (kind == SELECTION)
                .then(|| {
                    highlight_parent.or_else(|| self.retained_highlight_inheritance_parent_style_record(node, kind))
                })
                .flatten();
            let state = states[usize::from(kind)].filter(|_| has_rules);
            if has_rules && state.is_none() {
                counters.bump(Counter::EngineComputedRecordBailPseudoRow);
                return Err(Unanswered::Refused);
            }
            // The row has to hold the rules that flipped for this kind: one this flush published
            // holds the cascade of the node's current answer.
            if state.is_some()
                && scratch.flipped_pseudo_rules & (1_u64 << kind) != 0
                && self.current_winner_groups().pseudo_row_stamp(
                    node,
                    tree::PseudoElementTarget::new(tree::PseudoElementKind(u16::from(kind))),
                ) != Some(self.flush_stamp)
            {
                counters.bump(Counter::EngineComputedRecordBailPseudoFlip);
                return Err(Unanswered::Refused);
            }
            let old_record = old.unwrap_or(computed::FinalStyleRecordID::NONE);
            let remove = |engine: &mut Self, scratch: &mut EngineComputedRecordScratch, counters: &mut Counters| {
                if old.is_some() {
                    engine.note_engine_computed_pseudo_record(
                        node,
                        kind,
                        old_record,
                        computed::FinalStyleRecordID::NONE,
                        None,
                        0,
                        scratch,
                        counters,
                        !cssom_read,
                    );
                }
            };
            if !has_rules && !implicit && highlight_parent_record.is_none() && !cssom_read {
                remove(self, scratch, counters);
                continue;
            }
            // Reuse only when the originating element preserves every input the pseudo reads,
            // including display transformation and explicit inheritance of non-inherited values.
            if kind != SELECTION
                && old.is_some_and(|record| self.record_counter_environment_is_current(node, record))
                && originating_inputs_unchanged
                && !state.is_some_and(|state| self.state_tree_counting_key(node, state).1 != 0)
                && (old_element_record == Some(new_element_record)
                    || !state.is_some_and(|state| self.state_explicitly_inherits_non_inherited_property(node, state)))
            {
                let bound = self
                    .computed_group_sets
                    .cascade_state(target)
                    .or_else(|| self.computed_group_sets.pseudo_retained_cascade_state(node, kind));
                // Custom declarations resolve against registrations that move without the state,
                // and a winner written with `attr()` against attributes that move without it.
                let unchanged = match (state, bound) {
                    (Some(state), Some((bound_generation, bound_state))) => {
                        bound_generation == generation
                            && self.winner_groups.custom_declarations_of(state) == Default::default()
                            && !(self.custom_property_registrations_changed
                                && self.state_has_substitutions(node, state))
                            && !self.state_reads_attributes(node, state)
                            && self.winner_groups.states_are_semantically_equal(bound_state, state)
                    }
                    (None, None) => true,
                    _ => false,
                } && (kind != MARKER || old_is_list_item == new_is_list_item);
                if unchanged {
                    continue;
                }
            }
            if pin_old_composition {
                // C++ still reads the old pseudo style while installing the replacement.
                let old = old.expect("a composition has a style record");
                self.computed_group_sets.pin_style_record(old.raw());
                self.batch_pinned_compositions.push((node, old.raw()));
            }
            // A pseudo-element's own custom declarations resolve over its element's environment,
            // as an element's resolve over its parent's.
            let has_registered_declarations = self.declares_registered_custom_property(node, Some(kind), &inputs);
            let provisional_registered = has_registered_declarations
                .then(|| self.provisional_registered_value_context(Some(new_element_record), &inputs));
            let mut environment = self
                .engine_custom_property_environment_of(
                    node,
                    Some(kind),
                    element_environment,
                    &inputs,
                    provisional_registered,
                    counters,
                )
                .count_refusal(counters, Counter::EngineComputedRecordBailCustomProperties)?;
            // A store substituting `attr()` holds the element's attributes, which no other
            // element shares.
            let reads_attributes = state.is_some_and(|state| self.state_reads_attributes(node, state));
            let mut store = match state {
                Some(state) => match scratch
                    .pseudo_stores
                    .get(&(kind, state, environment))
                    .filter(|_| !reads_attributes)
                {
                    Some(store) => store.clone(),
                    None => {
                        let mut substituted = false;
                        let inheritance_environment = self.held_inheritance_environment(node, Some(kind));
                        let store = std::sync::Arc::new(
                            self.cascaded_store_for_state(
                                node,
                                state,
                                Some(kind),
                                environment,
                                inheritance_environment,
                                &mut substituted,
                                counters,
                            )
                            .or_refused()?,
                        );
                        if substituted {
                            scratch.substituted_states.insert((state, environment));
                        }
                        scratch.store_capacity_bytes += store.capacity_bytes();
                        if !reads_attributes {
                            scratch.pseudo_stores.insert((kind, state, environment), store.clone());
                        }
                        store
                    }
                },
                None => std::sync::Arc::new(WinnerStore::default()),
            };
            pseudo_uses_substitution |=
                state.is_some_and(|state| scratch.substituted_states.contains(&(state, environment)));
            let no_box = pseudo_content_generates_nothing(&store.view(self), kind);
            if no_box && !observe_without_box && !has_registered_declarations {
                remove(self, scratch, counters);
                continue;
            }
            // Container units resolve against the originating element's query containers, which
            // no other element shares.
            let container_unit_mask = store.container_relative_length_unit_mask(self);
            // What the record is derived from: the element's inherited style, display and
            // environment, and the element's record itself only when the state inherits a
            // non-inherited property from it.
            let key = (!has_registered_declarations
                && container_unit_mask == 0
                && !computed::ComputedGroupSets::record_is_animation_overlay(new_element_record.raw()))
            .then_some(())
            .and(
                self.computed_group_sets
                    .node_inherited_groups_identity(node)
                    .zip(self.box_type_parent_display(node))
                    .map(|(inherited_groups, parent_display)| PseudoCohortKey {
                        parent_record: if kind == SELECTION
                            || kind == BACKDROP
                            || state
                                .is_some_and(|state| self.state_explicitly_inherits_non_inherited_property(node, state))
                        {
                            new_element_record.raw()
                        } else {
                            0
                        },
                        highlight_parent_record: highlight_parent_record.map_or(0, |record| record.raw()),
                        inherited_groups,
                        parent_display,
                        dependency_flags: new_view_dependency_flags,
                        environment,
                        kind,
                        generation,
                        state,
                        facts,
                        font_environment_generation: inputs.font_environment_generation,
                        custom_property_registration_generation: inputs.custom_property_registration_generation,
                        root_font_inputs: RootFontInputs::from_document(&inputs),
                        substitution_attributes: state
                            .map_or(0, |state| self.substitution_attributes_key(node, Some(kind), state)),
                        tree_counting_key: state.map_or((0, 0), |state| self.state_tree_counting_key(node, state)),
                    }),
            );
            let cascade_state = state.map(|state| (generation, state));
            let own_groups = state.map_or(0, |state| self.state_owned_inherited_groups(state));
            let derived_under_element = |engine: &Self, record: computed::FinalStyleRecordID| {
                engine.computed_group_sets.final_style_record_is_live(record.raw())
                    && engine.record_counter_environment_is_current(node, record)
                    && engine
                        .computed_group_sets
                        .style_record_inherits_from_node(record.raw(), node, own_groups)
            };
            let shared = key.and_then(|key| {
                scratch
                    .pseudo_cohorts
                    .get(&key)
                    .copied()
                    .filter(|&record| derived_under_element(self, record))
                    .or_else(|| {
                        let record = *self.engine_pseudo_record_cache.get(&key)?;
                        derived_under_element(self, record).then_some(record)
                    })
            });
            let (new_style_record, longhand_evaluations) = match shared {
                Some(record) => {
                    if let Some(cascade_state) = cascade_state {
                        self.computed_group_sets
                            .set_pending_cascade_state(target, cascade_state);
                    }
                    let publication = self.assign_shared_style_record(
                        target,
                        record.raw(),
                        computed::ENGINE_INHERITED_GROUP_COUNT,
                        false,
                        counters,
                    );
                    counters.bump(Counter::EngineComputedRecordCohortHits);
                    (publication.style_record_identity, 0)
                }
                None => {
                    let subject = DriveSubject {
                        target: crate::css::style::computed::ComputedStyleTarget::new(node, kind),
                        parent: Some(node),
                        facts,
                        highlight_parent: highlight_parent_record,
                    };
                    let mut explicitly_inherited_groups = 0;
                    let driven = self.engine_full_drive(
                        subject,
                        None,
                        &store,
                        &inputs,
                        &mut scratch.font_drive,
                        FontDriveGoal::Complete,
                        has_registered_declarations,
                        &mut explicitly_inherited_groups,
                        counters,
                    );
                    let driven = if let Ok(FullDrive::AwaitsRegisteredContext(registered)) = driven {
                        environment = self.engine_custom_property_environment_of(
                            node,
                            Some(kind),
                            element_environment,
                            &inputs,
                            Some(registered),
                            counters,
                        )?;
                        let mut substituted = false;
                        let inheritance_environment = self.held_inheritance_environment(node, Some(kind));
                        let final_store = self
                            .cascaded_store_for_state(
                                node,
                                state.or_refused()?,
                                Some(kind),
                                environment,
                                inheritance_environment,
                                &mut substituted,
                                counters,
                            )
                            .or_refused()?;
                        scratch.store_capacity_bytes += final_store.capacity_bytes();
                        store = std::sync::Arc::new(final_store);
                        pseudo_uses_substitution |= substituted;
                        self.engine_full_drive(
                            subject,
                            None,
                            &store,
                            &inputs,
                            &mut scratch.font_drive,
                            FontDriveGoal::Complete,
                            false,
                            &mut explicitly_inherited_groups,
                            counters,
                        )
                    } else {
                        driven
                    };
                    if matches!(driven, Err(Unanswered::Suspended(Suspension::Font))) {
                        scratch.next_pseudo = pseudo_index;
                        scratch.pseudo_uses_substitution = pseudo_uses_substitution;
                    }
                    let (table, length, longhand_evaluations, font) = match driven? {
                        FullDrive::Driven(driven) => driven,
                        FullDrive::AwaitsRegisteredContext(_) | FullDrive::RootInputs(_) => {
                            unreachable!("a complete drive resumed with its registered context finishes")
                        }
                    };
                    if selected_kind.is_none()
                        && has_registered_declarations
                        && pseudo_content_generates_nothing(&store.view(self), kind)
                    {
                        remove(self, scratch, counters);
                        continue;
                    }
                    if explicitly_inherited_groups != 0 && kind != SELECTION {
                        scratch.pseudo_explicitly_inherited_groups |= explicitly_inherited_groups;
                    }
                    let font = font.expect("a full drive resolves the font");
                    let (record, _) = self.assemble_and_publish_engine_record(
                        target,
                        true,
                        Some(new_element_record),
                        table,
                        &length,
                        &font,
                        environment,
                        0,
                        0,
                        cascade_state,
                        &mut scratch.computability,
                        counters,
                    );
                    if let Some(key) = key {
                        scratch.pseudo_cohorts.insert(key, record);
                        if self.engine_pseudo_record_cache.len() >= COLD_RECORD_CACHE_LIMIT {
                            self.engine_pseudo_record_cache.clear();
                        }
                        self.engine_pseudo_record_cache.insert(key, record);
                    }
                    (record, longhand_evaluations)
                }
            };
            // The host records what the pseudo-element's container units read as the element's own.
            self.note_container_unit_effects_for_host(node, new_style_record, container_unit_mask);
            if self.pseudo_owes_css_animation_plan(node, kind, state) {
                // The record is one the engine assembled, which always holds its table.
                let table = self
                    .computed_group_sets
                    .style_record_view(new_style_record.raw())
                    .and_then(|view| unsafe { view.longhand_table.as_ref() });
                debug_assert!(table.is_some(), "an engine-assembled record without a longhand table");
                if let Some(table) = table {
                    let declaration_scope = state.and_then(|state| self.animation_name_declaration_scope(node, state));
                    let plan = self.settled_animation_plan(node, kind, table, declaration_scope);
                    self.nodes_owing_animation_definitions.insert((node, kind), plan);
                }
            }
            self.note_engine_computed_pseudo_record(
                node,
                kind,
                old_record,
                new_style_record,
                cascade_state,
                longhand_evaluations,
                scratch,
                counters,
                !cssom_read && !no_box,
            );
        }
        if scratch.pseudo_explicitly_inherited_groups != 0 {
            *self.nodes_owing_explicit_inheritance.entry(node).or_default() |=
                scratch.pseudo_explicitly_inherited_groups;
            scratch.pseudo_explicitly_inherited_groups = 0;
        }
        scratch.pseudo_uses_substitution = pseudo_uses_substitution;
        Ok(())
    }

    pub(super) fn drop_demand_pseudo_record(&mut self, node: StyleNodeID, kind: u8) {
        if let Some(record) = self.demand_pseudo_records.remove(&(node, kind)) {
            self.computed_group_sets.unpin_style_record(record.raw());
        }
    }

    /// Account for a pseudo-element record the engine settled (a removal when `new_style_record`
    /// is none) and leave its commitment to C++'s acknowledgement of the element.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn note_engine_computed_pseudo_record(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        old_style_record: computed::FinalStyleRecordID,
        new_style_record: computed::FinalStyleRecordID,
        cascade_state: Option<(u64, CascadeStateID)>,
        longhand_evaluations: u32,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
        count_record: bool,
    ) {
        if count_record {
            counters.bump(Counter::EngineComputedPseudoRecords);
        }
        self.engine_computed_records_pending
            .entry(node)
            .or_default()
            .push(PendingEngineComputedRecord {
                node,
                pseudo_kind,
                old_style_record,
                new_style_record,
                cascade_state,
                longhand_evaluations,
            });
        scratch.pseudo_deltas.push(PseudoRecordDelta {
            kind: pseudo_kind,
            old_style_record,
            new_style_record,
        });
    }

    /// Put a pseudo-element back the way it was before the engine settled it, unless a
    /// publication has moved it on since.
    pub(super) fn revert_engine_computed_pseudo_record(
        &mut self,
        pending: &PendingEngineComputedRecord,
        counters: &mut Counters,
    ) {
        // A removal is applied only on acknowledgement.
        if pending.new_style_record == computed::FinalStyleRecordID::NONE {
            return;
        }
        let target = computed::ComputedStyleTarget::new(pending.node, pending.pseudo_kind);
        if self
            .computed_group_sets
            .pseudo_style_record(pending.node, pending.pseudo_kind)
            != Some(pending.new_style_record)
        {
            return;
        }
        self.computed_group_sets.take_pending_cascade_state(target);
        if pending.old_style_record != computed::FinalStyleRecordID::NONE
            && self
                .computed_group_sets
                .final_style_record_is_live(pending.old_style_record.raw())
        {
            self.assign_shared_style_record(
                target,
                pending.old_style_record.raw(),
                computed::ENGINE_INHERITED_GROUP_COUNT,
                false,
                counters,
            );
        } else {
            self.computed_group_sets
                .remove_pseudo(pending.node, pending.pseudo_kind);
        }
    }

    pub(super) fn pseudo_style_mask(&self, node: StyleNodeID) -> Option<u64> {
        let bit = |pseudo: Option<tree::PseudoElementTarget>| {
            pseudo
                .map(|pseudo| pseudo.kind.0)
                .filter(|&kind| kind <= bridge::LAST_SYNTHETIC_PSEUDO_ELEMENT_KIND)
                .map_or(0, |kind| 1u64 << kind)
        };
        if let Some((owner, answer)) = Self::published_answer_lookup(
            &self.published_match_answers,
            self.batch_matching_traversal.as_deref(),
            node,
        ) {
            if let Some(identity) = answer.cascade_input {
                return self.match_answers.synthetic_pseudo_mask(identity);
            }
            if let Some(matches) = owner.matches_for(answer) {
                return Some(
                    matches
                        .iter()
                        .fold(0, |mask, rule_match| mask | bit(rule_match.pseudo_element)),
                );
            }
        }
        if let Some(mask) = self.computed_group_sets.node_pseudo_style_mask(node) {
            return Some(mask);
        }
        let identity = self.current_answer_identity(node)?;
        self.match_answers.answer(identity)?;
        self.match_answers.synthetic_pseudo_mask(identity)
    }

    /// One attempt at the synthetic pseudo-elements of an element whose record C++ computed: the
    /// engine settles them against that record exactly as it settles them beside one of its own.
    /// A resumed attempt has already passed the entry checks.
    fn settle_pseudo_records_after_host_record_step(
        &mut self,
        node: StyleNodeID,
        old_is_list_item: bool,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Drive<computed::FinalStyleRecordID> {
        self.drop_demand_pseudo_records(node);
        let record = self
            .computed_group_sets
            .sampled_composition_identity_for_pseudo(node)
            .and_then(computed::FinalStyleRecordID::from_raw)
            .or_else(|| self.computed_group_sets.assigned_style_record(node))
            .or_refused()?;
        if !scratch.font_drive.is_pending() {
            // A record the engine derived for the element itself carries its pseudo-elements,
            // and an element standing for its host's pseudo-element is that pseudo-element.
            if (self.engine_computed_records_pending.contains_key(&node)
                && self
                    .computed_group_sets
                    .sampled_composition_identity_for_pseudo(node)
                    .is_none())
                || self.computed_group_sets.adjustment_facts(node)
                    & bridge::element_adjustment_fact::IS_SHADOW_HOST_PSEUDO_ELEMENT
                    != 0
            {
                return Err(Unanswered::Refused);
            }
            // Every declaration the pseudo-elements' rules make has to be a winner the engine
            // holds, as it has for any record it derives. What the element's own declarations
            // make is in the record C++ computed.
            if !self.pseudo_winners_are_complete(node) {
                counters.bump(Counter::EngineComputedRecordBailIncompleteWinners);
                return Err(Unanswered::Refused);
            }
            if !self.engine_pseudo_inputs_available(node, Some(record), counters) {
                return Err(Unanswered::Refused);
            }
        }
        let generation = self.winner_groups.generation();
        if let Err(unanswered) = self.settle_engine_pseudo_records(
            node,
            None,
            Some(old_is_list_item),
            record,
            generation,
            scratch,
            counters,
            None,
            false,
            None,
            false,
        ) {
            if unanswered != Unanswered::Suspended(Suspension::Font) {
                // The element's record may be an engine answer awaiting acknowledgement. Only
                // pseudo records derived beside it go back when pseudo settlement fails.
                for pending in self.engine_computed_records_pending.remove(&node).into_iter().flatten() {
                    if pending.pseudo_kind == u8::MAX {
                        self.engine_computed_records_pending
                            .entry(node)
                            .or_default()
                            .push(pending);
                        continue;
                    }
                    let derived = pending.new_style_record;
                    self.revert_engine_computed_pseudo_record(&pending, counters);
                    scratch.pseudo_cohorts.retain(|_, record| *record != derived);
                    self.engine_pseudo_record_cache.retain(|_, record| *record != derived);
                }
                scratch.pseudo_deltas.clear();
                self.settle_computed_memory();
            }
            return Err(unanswered);
        }
        Ok(record)
    }

    /// Whether the node is an element standing for its shadow host's pseudo-element (the element
    /// a `::placeholder` or a slider part is): its style is that pseudo-element's, cascaded from
    /// the host's rules, and its own cascade decides nothing.
    pub(crate) fn backs_host_pseudo_element(&self, node: StyleNodeID) -> bool {
        self.backed_host_pseudo_element(node).is_some()
    }

    /// The pseudo-element kind an element stands for and the shadow host it stands for it on. The
    /// host publishes the kind with the element's facts, and an element in no shadow tree stands
    /// for nothing.
    pub(super) fn backed_host_pseudo_element(&self, node: StyleNodeID) -> Option<(u8, StyleNodeID)> {
        let kind = self.computed_group_sets.associated_pseudo_kind(node)?;
        Some((kind, self.tree.shadow_host_of(node)?))
    }

    /// The host's matches for the pseudo-element an element stands for, when the element's record
    /// can be cascaded from them. The backing element's published answer proves the inventory
    /// used to compute its style, including the host's pseudo-element rules.
    fn backing_element_rule_matches(
        &mut self,
        host: StyleNodeID,
        target: tree::PseudoElementTarget,
        backing_answer_is_complete: bool,
        counters: &mut Counters,
    ) -> Option<Vec<RuleMatch>> {
        let batch_matches = self.batch_backing_pseudo_matches.get(&host).map(|matches| {
            matches
                .iter()
                .filter(|entry| entry.pseudo_element == Some(target))
                .copied()
                .collect()
        });
        let matches: Vec<RuleMatch> = if let Some(matches) = batch_matches {
            matches
        } else {
            match Self::published_answer_lookup(
                &self.published_match_answers,
                self.batch_matching_traversal.as_deref(),
                host,
            )
            .and_then(|(published, answer)| {
                if let Some(matches) = published.matches_for(answer) {
                    return Some(
                        matches
                            .iter()
                            .filter(|entry| entry.pseudo_element == Some(target))
                            .copied()
                            .collect(),
                    );
                }
                self.match_answers
                    .answer(answer.cascade_input?)?
                    .iter()
                    .filter(|entry| {
                        self.programs.get(entry.program).entries()[entry.entry as usize].pseudo_element == Some(target)
                    })
                    .map(|entry| entry.materialize(host, &self.programs, 0))
                    .collect::<Option<Vec<_>>>()
            }) {
                Some(matches) => matches,
                None => match self.retained_match_answer(host) {
                    Lookup::Known(answer) => answer
                        .iter()
                        .filter(|entry| {
                            self.programs.get(entry.program).entries()[entry.entry as usize].pseudo_element
                                == Some(target)
                        })
                        .map(|entry| entry.materialize(host, &self.programs, 0))
                        .collect::<Option<_>>()?,
                    _ => self
                        .exact_match_answer(host, counters)
                        .ok()?
                        .into_iter()
                        .filter(|entry| entry.pseudo_element == Some(target))
                        .collect(),
                },
            }
        };
        backing_answer_is_complete.then_some(matches)
    }

    /// The matches for element-backed pseudo-elements in the answer a transaction publishes for a
    /// node, read before that answer is installed. `None` when it has none.
    pub(in crate::css::style) fn batch_backing_pseudo_matches_of(
        &self,
        node: StyleNodeID,
        published: &PublishedMatchAnswers,
        answer: &PublishedMatchAnswer,
    ) -> Option<Vec<RuleMatch>> {
        let is_backed = |pseudo: Option<tree::PseudoElementTarget>| {
            pseudo.is_some_and(|pseudo| {
                (u16::from(bridge::FIRST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND)
                    ..=u16::from(bridge::LAST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND))
                    .contains(&pseudo.kind.0)
            })
        };
        let matches: Vec<RuleMatch> = match published.matches_for(answer) {
            Some(matches) => matches
                .iter()
                .filter(|entry| is_backed(entry.pseudo_element))
                .copied()
                .collect(),
            None => self
                .match_answers
                .answer(answer.cascade_input?)?
                .iter()
                .filter(|entry| {
                    is_backed(self.programs.get(entry.program).entries()[entry.entry as usize].pseudo_element)
                })
                .map(|entry| entry.materialize(node, &self.programs, 0))
                .collect::<Option<_>>()?,
        };
        (!matches.is_empty()).then_some(matches)
    }

    /// The record of an element standing for its shadow host's pseudo-element, the way C++
    /// computes one: the host's rules for that pseudo-element cascaded with the element's own
    /// declarations, the rules' custom declarations resolved over the element's parent's
    /// environment, inheriting from the element's own flat-tree parent, with the element's own box
    /// adjustments. `None` leaves it to C++.
    pub(super) fn engine_backing_element_record(
        &mut self,
        node: StyleNodeID,
        (kind, host): (u8, StyleNodeID),
        backing_answer_is_complete: bool,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Drive<RecordDelta> {
        use bridge::element_adjustment_fact as fact;
        let facts = self.computed_group_sets.adjustment_facts(node);
        // A record it already holds is replaced by a full drive. The host samples the element's
        // animations over the new base and runs its transition step against the record it held;
        // transition declarations alone do not prevent driving the next base record, whose delta
        // lets the host start a transition. An animation overlay is the composition of the
        // element's own effects, which the sample makes again over the new base, or clears when
        // they have all gone.
        let old_record = self.computed_group_sets.assigned_style_record(node);
        let holds_an_overlay = match old_record {
            Some(old) => {
                let Some(view) = self.computed_group_sets.style_record_view(old.raw()) else {
                    counters.bump(Counter::EngineComputedRecordBailRecord);
                    return Err(Unanswered::Refused);
                };
                !view.animated_overlay.is_null()
            }
            None => false,
        };
        let animates = facts & fact::HAS_ANIMATIONS != 0 || holds_an_overlay;
        let target = tree::PseudoElementTarget::new(tree::PseudoElementKind(u16::from(kind)));
        // The host's rules for the pseudo-element, cascaded as the element's own with its own
        // declarations, as C++ cascades them for it.
        let Some(mut matches) = self.backing_element_rule_matches(host, target, backing_answer_is_complete, counters)
        else {
            counters.bump(Counter::EngineComputedRecordBailIncompleteWinners);
            return Err(Unanswered::Refused);
        };
        // The pseudo-element's custom declarations cascade from these matches too: a shadow host
        // retains no answer to read them from.
        let custom_declarations = self.cascade_custom_declarations(host, Some(kind), Some(&matches));
        for entry in &mut matches {
            entry.node = node;
            entry.pseudo_element = None;
        }
        let mut winners = self.resolved_cascade_winners_for_properties(node, &matches, None, None);
        // A pseudo-element takes from its rules only the properties it supports; the element's
        // own declarations are not held to that. An own declaration a dropped rule winner hid
        // wins among the element's own declarations alone, since no rule declares it for this
        // pseudo-element.
        let (declared_properties, _) = self
            .facts
            .element_declared_properties(node, ElementDeclarationKind::InlineStyle);
        let own_declared: Vec<u16> = declared_properties.iter().map(|declared| declared.property).collect();
        let mut hidden_own_declarations = Vec::new();
        winners.retain(|winner| {
            let supported = !matches!(winner.source, WinnerSource::Rule(_))
                || crate::css::property_metadata::pseudo_element_supports_property(kind, winner.property);
            if !supported && own_declared.contains(&winner.property) {
                hidden_own_declarations.push(winner.property);
            }
            supported
        });
        if !hidden_own_declarations.is_empty() {
            hidden_own_declarations.sort_unstable();
            hidden_own_declarations.dedup();
            winners.extend(self.resolved_cascade_winners_for_properties(
                node,
                &[],
                None,
                Some(&hidden_own_declarations),
            ));
            winners.sort_unstable_by_key(|winner| winner.property);
        }
        let state = self.with_cascade_interning_counters(|groups| groups.intern_sorted(&winners, None), counters);
        // The CSS animations the element's winners name, or the ones it still runs, take the
        // complete plan the drive's longhands settle, which the host applies before it samples.
        let owes_an_animation_plan =
            !self.state_has_no_animation_name(state) || self.css_defined_animations.node_runs_a_css_animation(node);
        let Some(mut inputs) = self.document_style_computation_inputs else {
            counters.bump(Counter::EngineComputedRecordBailNoEnvironment);
            return Err(Unanswered::Refused);
        };
        if let Some((root, root_inputs)) = scratch.root_element_inputs
            && root == node
        {
            root_inputs.apply_to(&mut inputs);
        }
        let parent = self.tree.flat_tree_parent(node).or_refused()?;
        let (Some(parent_record), Some(parent_environment)) = (
            self.computed_group_sets.assigned_style_record(parent),
            self.computed_group_sets.custom_property_environment_identity(parent),
        ) else {
            counters.bump(Counter::EngineComputedRecordBailRecordParent);
            return Err(Unanswered::Refused);
        };
        let Some(custom_declarations) = custom_declarations else {
            counters.bump(Counter::EngineComputedRecordBailCustomProperties);
            return Err(Unanswered::Refused);
        };
        let has_registered_declarations =
            self.declarations_name_a_registered_custom_property(&custom_declarations, &inputs);
        let provisional_registered = has_registered_declarations
            .then(|| self.provisional_registered_value_context(Some(parent_record), &inputs));
        let mut environment = self
            .engine_custom_property_environment_over(
                host,
                Some(kind),
                custom_declarations.clone(),
                parent_environment,
                &inputs,
                provisional_registered,
                counters,
            )
            .count_refusal(counters, Counter::EngineComputedRecordBailCustomProperties)?;
        let mut substituted = false;
        let inheritance_environment = Some(parent_environment);
        let mut store = self
            .cascaded_store_for_state(
                node,
                state,
                None,
                environment,
                inheritance_environment,
                &mut substituted,
                counters,
            )
            .or_refused()?;
        let Some(pseudo_styles) = self.pseudo_style_mask(node) else {
            counters.bump(Counter::EngineComputedRecordBailPseudoMask);
            return Err(Unanswered::Refused);
        };
        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        let subject = DriveSubject {
            target,
            parent: Some(parent),
            facts,
            highlight_parent: None,
        };
        let mut explicitly_inherited_groups = 0;
        let driven = self.engine_full_drive(
            subject,
            None,
            &store,
            &inputs,
            &mut scratch.font_drive,
            FontDriveGoal::Complete,
            has_registered_declarations,
            &mut explicitly_inherited_groups,
            counters,
        )?;
        let driven = if let FullDrive::AwaitsRegisteredContext(registered) = driven {
            environment = self.engine_custom_property_environment_over(
                host,
                Some(kind),
                custom_declarations,
                parent_environment,
                &inputs,
                Some(registered),
                counters,
            )?;
            substituted = false;
            store = self
                .cascaded_store_for_state(
                    node,
                    state,
                    None,
                    environment,
                    inheritance_environment,
                    &mut substituted,
                    counters,
                )
                .or_refused()?;
            self.engine_full_drive(
                subject,
                None,
                &store,
                &inputs,
                &mut scratch.font_drive,
                FontDriveGoal::Complete,
                false,
                &mut explicitly_inherited_groups,
                counters,
            )?
        } else {
            driven
        };
        let (table, length, longhand_evaluations, font) = match driven {
            FullDrive::Driven(driven) => driven,
            FullDrive::AwaitsRegisteredContext(_) | FullDrive::RootInputs(_) => {
                unreachable!("a complete drive resumed with its registered context finishes")
            }
        };
        let font = font.expect("a full drive resolves the font");
        let animation_plan = owes_an_animation_plan.then(|| {
            self.settled_animation_plan(
                node,
                u8::MAX,
                &table,
                self.animation_name_declaration_scope(node, state),
            )
        });
        // The transition step reads the composition the element held as its before-change style,
        // after the new base replaced it.
        let old_composition =
            old_record.filter(|old| animates && computed::ComputedGroupSets::record_is_animation_overlay(old.raw()));
        if let Some(old) = old_composition {
            self.computed_group_sets.pin_style_record(old.raw());
        }
        let (record, _) = self.assemble_and_publish_engine_record(
            target,
            true,
            Some(parent_record),
            table,
            &length,
            &font,
            environment,
            pseudo_styles,
            0,
            None,
            &mut scratch.computability,
            counters,
        );
        if let Some(old) = old_composition {
            self.batch_pinned_compositions.push((node, old.raw()));
        }
        let _ = longhand_evaluations;
        if explicitly_inherited_groups != 0 {
            self.nodes_owing_explicit_inheritance
                .insert(node, explicitly_inherited_groups);
        }
        if let Some(plan) = animation_plan {
            self.nodes_owing_animation_definitions.insert((node, u8::MAX), plan);
        }
        if animates || owes_an_animation_plan {
            self.nodes_owing_an_animation_sample.insert(node);
        }
        counters.bump(Counter::EngineComputedRecordHostPseudoBackings);
        scratch.noted_substitution = Some(substituted);
        Ok((old_record.unwrap_or(computed::FinalStyleRecordID::NONE), record))
    }

    /// Whether every rule the node's answer matches for a pseudo-element declares only what the
    /// winner columns hold, and custom properties, which the engine resolves into the
    /// pseudo-element's own environment, including a container verdict held with its origin.
    pub(super) fn pseudo_winners_are_complete(&self, node: StyleNodeID) -> bool {
        let rule_is_complete = |rule: RuleID| {
            self.container_gate_is_held(Some(node), rule, true)
                && self.program.declarations_are_complete_but_for_custom_properties(rule)
        };
        if let Some((published, answer)) = Self::published_answer_lookup(
            &self.published_match_answers,
            self.batch_matching_traversal.as_deref(),
            node,
        ) && let Some(matches) = published.matches_for(answer)
        {
            return matches
                .iter()
                .filter(|entry| entry.pseudo_element.is_some())
                .all(|entry| rule_is_complete(entry.rule));
        }
        let Lookup::Known(answer) = self.retained_match_answer(node) else {
            return false;
        };
        answer.iter().all(|rule_match| {
            self.programs.get(rule_match.program).entries()[rule_match.entry as usize]
                .pseudo_element
                .is_none()
                || rule_is_complete(rule_match.rule)
        })
    }
}

/// Whether a pseudo-element's winning `content` generates no box: `none` for every kind, and
/// `normal` (the initial value, so also an absent one) for ::before and ::after.
fn pseudo_content_generates_nothing(store: &impl crate::css::cascaded_properties::CascadedValues, kind: u8) -> bool {
    use crate::css::property_metadata::property_id as prop;
    use crate::css::style_compute::keyword;
    let generated = matches!(kind, pseudo_kind::BEFORE | pseudo_kind::AFTER);
    match store
        .winning_declaration(prop::CONTENT)
        .map(|(value, ..)| unsafe { &*value.cast::<StyleValueData>() })
    {
        None => generated,
        Some(StyleValueData::Keyword { keyword }) => {
            *keyword == keyword::NONE || (*keyword == keyword::NORMAL && generated)
        }
        Some(_) => false,
    }
}

impl StyleEngineState {
    pub(super) fn demand_pseudo_record(
        &mut self,
        node: StyleNodeID,
        kind: u8,
        read_only: bool,
        targeted: bool,
        highlight_parent: Option<computed::FinalStyleRecordID>,
        counters: &mut Counters,
    ) -> Result<Option<computed::FinalStyleRecordID>, &'static str> {
        let cssom_read = read_only && !targeted;
        // A kind the engine holds no rows for generates no box.
        if kind >= 20 {
            return Ok(None);
        }
        let Some(mask) = self.pseudo_style_mask(node) else {
            return Err("EngineComputedRecordBailPseudoMask");
        };
        let element = self
            .computed_group_sets
            .assigned_style_record(node)
            .ok_or("EngineComputedRecordBailRecord")?;
        let is_list_item = self
            .computed_group_sets
            .style_record_view(element.raw())
            .and_then(|view| unsafe { view.longhand_table.as_ref() })
            .is_some_and(|table| table.display_is_list_item());
        // CSSOM reads still need computed values for an ungenerated pseudo-element. Derive a
        // private record from the originating element without publishing a generated box.
        // Targeted settlement observes generated records only, while keeping the demand private.
        let cssom_absent = cssom_read
            && (mask & (1 << kind) == 0 || kind == pseudo_kind::MARKER && !is_list_item)
            && self.document_style_computation_inputs.is_some();
        if !cssom_absent
            && mask & (1 << kind) == 0
            && !(kind == pseudo_kind::MARKER && is_list_item)
            && !(kind == pseudo_kind::SELECTION
                && self
                    .retained_highlight_inheritance_parent_style_record(node, kind)
                    .is_some())
        {
            self.drop_demand_pseudo_record(node, kind);
            return Ok(None);
        }
        if !self.pseudo_winners_are_complete(node) {
            return Err("EngineComputedRecordBailIncompleteWinners");
        }
        let before = counters.record_bail_marks();
        let mut scratch = EngineComputedRecordScratch::default();
        let generation = self.winner_groups.generation();
        loop {
            match self.settle_engine_pseudo_records(
                node,
                (!cssom_absent).then_some(element),
                None,
                element,
                generation,
                &mut scratch,
                counters,
                Some(kind),
                cssom_absent,
                highlight_parent,
                cssom_read,
            ) {
                Ok(()) => break,
                Err(Unanswered::Suspended(Suspension::RandomBases)) => self.refill_random_base_requests(),
                Err(Unanswered::Suspended(Suspension::Font)) => {
                    let request = scratch.font_drive.take_suspended_request();
                    self.refill_font_requests(vec![(Some(node), request)], counters);
                }
                Err(Unanswered::Refused) => {
                    return Err(counters
                        .first_changed_record_bail(&before)
                        .unwrap_or("ComputationBailUnnamed"));
                }
            }
        }
        let record = match scratch.pseudo_deltas.iter().rev().find(|delta| delta.kind == kind) {
            Some(delta) if delta.new_style_record == computed::FinalStyleRecordID::NONE => None,
            Some(delta) => Some(delta.new_style_record),
            None => self.computed_group_sets.pseudo_style_record(node, kind),
        };
        if let Some(record) = record {
            self.computed_group_sets.pin_style_record(record.raw());
        }
        for pending in self.engine_computed_records_pending.remove(&node).into_iter().flatten() {
            if pending.pseudo_kind == kind {
                self.revert_engine_computed_pseudo_record(&pending, counters);
            } else {
                self.engine_computed_records_pending
                    .entry(node)
                    .or_default()
                    .push(pending);
            }
        }
        self.drop_demand_pseudo_record(node, kind);
        if let Some(record) = record {
            self.demand_pseudo_records.insert((node, kind), record);
        }
        self.settle_computed_memory();
        Ok(record)
    }

    /// Settle the synthetic pseudo-elements of an element whose record C++ has just computed and
    /// installed, so C++ installs the engine's records for them instead of computing each one:
    /// their inputs are the element's record and the published winner states, all current once
    /// the element's own computation has published its record. `old_is_list_item` is whether the
    /// element generated a marker before. A zero `style_record` leaves the pseudo-elements to C++;
    /// the flag says whether a settled one substituted custom properties.
    pub(crate) fn settle_pseudo_records_after_host_record(
        &mut self,
        node: StyleNodeID,
        old_is_list_item: bool,
        counters: &mut Counters,
    ) -> (RetriedEngineRecord, bool) {
        if self
            .retained
            .computed_group_sets
            .sampled_composition_identity_for_pseudo(node)
            .is_some()
            && let Some(pending) = self.retained.engine_computed_records_pending.remove(&node)
        {
            let mut element_pending = Vec::new();
            for record in pending {
                if record.pseudo_kind == u8::MAX {
                    element_pending.push(record);
                } else {
                    self.retained.revert_engine_computed_pseudo_record(&record, counters);
                }
            }
            if !element_pending.is_empty() {
                self.retained
                    .engine_computed_records_pending
                    .insert(node, element_pending);
            }
        }
        if let Some(inputs) = self.retained.document_style_computation_inputs
            && let Some(resolver) = &mut self.retained.font_resolution
        {
            resolver.prepare(inputs.font_environment_generation);
        }
        let mut scratch = EngineComputedRecordScratch::default();
        let mut suspended_memory = MemoryLease::new(MemoryCategory::BatchScratch);
        let record = loop {
            let record = self.retained.settle_pseudo_records_after_host_record_step(
                node,
                old_is_list_item,
                &mut scratch,
                counters,
            );
            let Err(Unanswered::Suspended(Suspension::Font)) = record else {
                break record;
            };
            let request = scratch.font_drive.take_suspended_request();
            suspended_memory.resize_required_to(&mut self.memory, scratch.font_drive.capacity_bytes());
            self.refill_font_requests(vec![(Some(node), request)], counters);
        };
        let mut settled = RetriedEngineRecord::default();
        let Ok(record) = record else {
            counters.bump(Counter::EngineComputedRecordHostPseudoDeclines);
            return (settled, false);
        };
        counters.bump(Counter::EngineComputedRecordHostPseudoSettles);
        settled.style_record = record.raw();
        for delta in &scratch.pseudo_deltas {
            let kind = usize::from(delta.kind);
            if kind < bridge::RETRY_PSEUDO_RECORD_SLOTS {
                settled.pseudo_records_present |= 1 << kind;
                settled.pseudo_records[kind] = delta.new_style_record.raw();
            }
        }
        (settled, scratch.pseudo_uses_substitution)
    }
}
