/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

impl RetainedState {
    /// Check pseudo winner availability before deriving an originating record that would have
    /// to be discarded. Marker generation additionally depends on the newly computed display
    /// and is checked when settling the pseudo records.
    pub(super) fn engine_pseudo_inputs_available(
        &mut self,
        node: StyleNodeID,
        record: Option<computed::FinalStyleRecordID>,
        counters: &mut Counters,
    ) -> bool {
        use pseudo_kind::{AFTER, BACKDROP, BEFORE, FIRST_LETTER, MARKER, SELECTION};

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
            if kind >= pseudo_kind::SYNTHETIC_COUNT || kind == usize::from(BACKDROP) || pseudo_kind::is_highlight(kind)
            {
                continue;
            }
            if version != self.program.version() || !priority_current {
                if kind == usize::from(MARKER) && !marker_may_generate {
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
        required &= !pseudo_kind::highlight_mask();
        if required & !available == 0 {
            return true;
        }
        let mut explicit_kinds = (1 << BEFORE) | (1 << AFTER) | (1 << FIRST_LETTER) | (1 << SELECTION);
        if record_is_list_item || display_winner_is_list_item == Some(true) {
            explicit_kinds |= 1 << MARKER;
        }
        if required & explicit_kinds & !available != 0 {
            counters.bump(Counter::EngineComputedRecordBailPseudoRow);
            return false;
        }
        true
    }

    pub(super) fn engine_marker_font_supported(&mut self, node: StyleNodeID, counters: &mut Counters) -> bool {
        // Reject unsupported existing marker fonts before computing the originating element.
        // A later change to supported settings still computes correctly through C++ and makes
        // the next attempt eligible. The default marker's tabular numerals are not supported by
        // the engine font resolver yet.
        if let Some(marker) = self.computed_group_sets.pseudo_style_record(node, pseudo_kind::MARKER)
            && let Some(view) = self.computed_group_sets.style_record_view(marker.raw())
            && let Some(table) = unsafe { view.longhand_table.as_ref() }
        {
            let value = table
                .effective_value(
                    None,
                    crate::css::property_metadata::property_id::FONT_VARIANT_NUMERIC,
                    true,
                )
                .value;
            if !matches!(unsafe { value.cast::<StyleValueData>().as_ref() },
                Some(StyleValueData::Keyword { keyword }) if *keyword == crate::css::style_compute::keyword::NORMAL)
            {
                counters.bump(Counter::EngineComputedRecordBailFontPhase);
                return false;
            }
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
    ) -> Option<()> {
        self.settle_engine_pseudo_records(
            node,
            old_element_record,
            None,
            new_element_record,
            generation,
            scratch,
            counters,
        )
    }

    /// `old_is_list_item` says whether the element was a list item when the old record it no
    /// longer holds is unknown: C++ computed the new one over it.
    #[allow(clippy::too_many_arguments)]
    fn settle_engine_pseudo_records(
        &mut self,
        node: StyleNodeID,
        old_element_record: Option<computed::FinalStyleRecordID>,
        old_is_list_item: Option<bool>,
        new_element_record: computed::FinalStyleRecordID,
        generation: u64,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Option<()> {
        use pseudo_kind::{AFTER, BACKDROP, BEFORE, FIRST_LETTER, MARKER, SELECTION};

        let Some(mut inputs) = self.document_style_computation_inputs else {
            counters.bump(Counter::EngineComputedRecordBailNoEnvironment);
            return None;
        };
        // NB: Root pseudos use the originating record's current font, independently of
        //     the document context used for the root's own remaining properties.
        if self.computed_group_sets.adjustment_facts(node) & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT != 0 {
            self.root_font_inputs_from_record(new_element_record)?
                .apply_to(&mut inputs);
        }
        let program_version = self.program.version();
        let mut states: [Option<CascadeStateID>; pseudo_kind::SYNTHETIC_COUNT] = [None; pseudo_kind::SYNTHETIC_COUNT];
        let mut marker_row_is_stale = false;
        for (pseudo, version, state, priority_current) in self.current_winner_groups().pseudo_states(node) {
            if self.deferred_pseudo_element == Some(pseudo.kind) {
                continue;
            }
            let Ok(kind) = u8::try_from(pseudo.kind.0) else {
                continue;
            };
            if usize::from(kind) >= pseudo_kind::SYNTHETIC_COUNT {
                continue;
            }
            // A ::backdrop is materialized for a top-layer element only, which C++ decides; the
            // rules for it match every element. A stale row is no answer. A highlight
            // pseudo-element inherits from its parent element's, which C++ settles as well.
            if kind == BACKDROP || pseudo_kind::is_highlight(usize::from(kind)) {
                continue;
            }
            if version != program_version || !priority_current {
                // Whether a marker is generated is known once the element's display is.
                if kind == MARKER {
                    marker_row_is_stale = true;
                    continue;
                }
                counters.bump(Counter::EngineComputedRecordBailPseudoStale);
                return None;
            }
            states[usize::from(kind)] = Some(state);
        }
        // An element holding a backdrop style is in the top layer: its backdrop is C++'s.
        if self
            .computed_group_sets
            .assigned_pseudo_kinds(node)
            .any(|kind| kind == BACKDROP)
        {
            counters.bump(Counter::EngineComputedRecordBailPseudoBackdrop);
            return None;
        }
        let display_is_list_item = |engine: &Self, record: computed::FinalStyleRecordID| -> Option<bool> {
            let view = engine.computed_group_sets.style_record_view(record.raw())?;
            let table = unsafe { view.longhand_table.as_ref() }?;
            Some(table.display_is_list_item())
        };
        let Some(new_is_list_item) = display_is_list_item(self, new_element_record) else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return None;
        };
        let Some(new_view_dependency_flags) = self
            .computed_group_sets
            .style_record_view(new_element_record.raw())
            .map(|view| view.dependency_flags)
        else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return None;
        };
        let old_is_list_item = match (old_is_list_item, old_element_record) {
            (Some(old_is_list_item), _) => old_is_list_item,
            (None, Some(record)) => {
                let Some(list_item) = display_is_list_item(self, record) else {
                    counters.bump(Counter::EngineComputedRecordBailRecord);
                    return None;
                };
                list_item
            }
            (None, None) => false,
        };
        if marker_row_is_stale && (new_is_list_item || old_is_list_item) {
            counters.bump(Counter::EngineComputedRecordBailPseudoStale);
            return None;
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
        let originating_inputs_unchanged = inherited_inputs_unchanged
            && !scratch.root_font_inputs_changed
            && !scratch.font_environment_moved
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
        let Some(element_environment) = self.computed_group_sets.custom_property_environment_identity(node) else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return None;
        };
        // The kinds the node's match answer has rules for: a winner row is published for each
        // the engine cascaded itself, and a kind with rules but no row is not decided.
        let Some(kinds_with_rules) = self.pseudo_style_mask(node) else {
            counters.bump(Counter::EngineComputedRecordBailPseudoMask);
            return None;
        };
        let mut pseudo_uses_substitution = scratch.pseudo_uses_substitution;
        for (pseudo_index, kind) in [BEFORE, AFTER, FIRST_LETTER, SELECTION, MARKER]
            .into_iter()
            .enumerate()
            .skip(scratch.next_pseudo)
        {
            scratch.next_pseudo = pseudo_index + 1;
            if self.deferred_pseudo_element == Some(tree::PseudoElementKind(u16::from(kind)))
                || pseudo_kind::is_highlight(usize::from(kind))
            {
                continue;
            }
            let target = computed::ComputedStyleTarget::new(node, kind);
            let old = self.computed_group_sets.pseudo_style_record(node, kind);
            if let Some(old) = old {
                let Some(view) = self.computed_group_sets.style_record_view(old.raw()) else {
                    counters.bump(Counter::EngineComputedRecordBailRecord);
                    return None;
                };
                let transitioning = (unsafe { view.longhand_table.as_ref() })
                    .is_some_and(crate::css::style_compute::has_active_transition_properties);
                if !view.animated_overlay.is_null() || transitioning {
                    counters.bump(Counter::EngineComputedRecordBailRecordOverlay);
                    return None;
                }
            }
            // A marker is generated for a list item (and refreshed once more for an element that
            // stops being one), whatever rules match ::marker; other kinds are generated by their
            // rules, which the answer names: a winner row outlives the last rule as an empty
            // state, and a rule without declarations is an empty state that generates.
            let implicit = kind == MARKER && (new_is_list_item || old_is_list_item);
            if kind == MARKER && !implicit {
                continue;
            }
            let has_rules = kinds_with_rules & (1 << kind) != 0;
            let state = states[usize::from(kind)].filter(|_| has_rules);
            if has_rules && state.is_none() {
                counters.bump(Counter::EngineComputedRecordBailPseudoRow);
                return None;
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
                return None;
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
                    );
                }
            };
            if !has_rules && !implicit {
                remove(self, scratch, counters);
                continue;
            }
            // Reuse only when the originating element preserves every input the pseudo reads,
            // including display transformation and explicit inheritance of non-inherited values.
            if old.is_some()
                && originating_inputs_unchanged
                && (old_element_record == Some(new_element_record)
                    || !state.is_some_and(|state| self.state_explicitly_inherits_non_inherited_property(node, state)))
            {
                let bound = self
                    .computed_group_sets
                    .cascade_state(target)
                    .or_else(|| self.computed_group_sets.pseudo_retained_cascade_state(node, kind));
                // Custom declarations resolve against registrations that move without the state.
                let unchanged = match (state, bound) {
                    (Some(state), Some((bound_generation, bound_state))) => {
                        bound_generation == generation
                            && self.winner_groups.custom_declarations_of(state) == Default::default()
                            && self.winner_groups.states_are_semantically_equal(bound_state, state)
                    }
                    (None, None) => true,
                    _ => false,
                } && (kind != MARKER || old_is_list_item == new_is_list_item);
                if unchanged {
                    continue;
                }
            }
            // A pseudo-element's own custom declarations resolve over its element's environment,
            // as an element's resolve over its parent's.
            let Some(environment) =
                self.engine_custom_property_environment_of(node, Some(kind), element_environment, &inputs, counters)
            else {
                counters.bump(Counter::EngineComputedRecordBailCustomProperties);
                return None;
            };
            let store = match state {
                Some(state) => match scratch.pseudo_stores.get(&(kind, state, environment)) {
                    Some(store) => store.clone(),
                    None => {
                        let mut substituted = false;
                        let store = std::sync::Arc::new(self.cascaded_store_for_state(
                            node,
                            state,
                            Some(kind),
                            environment,
                            &mut substituted,
                            counters,
                        )?);
                        if substituted {
                            scratch.substituted_states.insert((state, environment));
                        }
                        scratch.store_capacity_bytes += store.capacity_bytes();
                        scratch.pseudo_stores.insert((kind, state, environment), store.clone());
                        store
                    }
                },
                None => std::sync::Arc::new(WinnerStore::default()),
            };
            pseudo_uses_substitution |=
                state.is_some_and(|state| scratch.substituted_states.contains(&(state, environment)));
            if pseudo_content_generates_nothing(&store.view(self), kind) {
                remove(self, scratch, counters);
                continue;
            }
            // What the record is derived from: the element's inherited style, display and
            // environment, and the element's record itself only when the state inherits a
            // non-inherited property from it.
            let key = self
                .computed_group_sets
                .node_inherited_groups_identity(node)
                .zip(self.box_type_parent_display(node))
                .map(|(inherited_groups, parent_display)| PseudoCohortKey {
                    parent_record: if state
                        .is_some_and(|state| self.state_explicitly_inherits_non_inherited_property(node, state))
                    {
                        new_element_record.raw()
                    } else {
                        0
                    },
                    inherited_groups,
                    parent_display,
                    dependency_flags: new_view_dependency_flags,
                    environment,
                    kind,
                    generation,
                    state,
                    facts,
                    font_environment_generation: inputs.font_environment_generation,
                    root_font_inputs: RootFontInputs::from_document(&inputs),
                });
            let cascade_state = state.map(|state| (generation, state));
            let own_groups = state.map_or(0, |state| self.state_owned_inherited_groups(state));
            let derived_under_element = |engine: &Self, record: computed::FinalStyleRecordID| {
                engine.computed_group_sets.final_style_record_is_live(record.raw())
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
                        recascade_node: None,
                        parent: Some(node),
                        facts,
                    };
                    let mut explicitly_inherited_groups = 0;
                    let driven = self.engine_full_drive(
                        subject,
                        None,
                        &store,
                        &inputs,
                        &mut scratch.font_drive,
                        FontDriveGoal::Complete,
                        &mut explicitly_inherited_groups,
                        counters,
                    );
                    if driven.is_none() && scratch.font_drive.request.is_some() {
                        scratch.next_pseudo = pseudo_index;
                        scratch.pseudo_uses_substitution = pseudo_uses_substitution;
                    }
                    let (table, length, longhand_evaluations, font) = driven?;
                    // The mark a pseudo-element's explicit `inherit` leaves is the originating
                    // element's own, which this row does not answer for: it stays with C++.
                    if explicitly_inherited_groups != 0 {
                        counters.bump(Counter::EngineComputedRecordBailDrive);
                        return None;
                    }
                    let font = font.expect("a full drive resolves the font");
                    let (record, _) = self.assemble_and_publish_engine_record(
                        target,
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
                    )?;
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
            self.note_engine_computed_pseudo_record(
                node,
                kind,
                old_record,
                new_style_record,
                cascade_state,
                longhand_evaluations,
                scratch,
                counters,
            );
        }
        scratch.pseudo_uses_substitution = pseudo_uses_substitution;
        Some(())
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
    ) {
        counters.bump(Counter::EngineComputedPseudoRecords);
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
        if let Some(&mask) = self.batch_pseudo_style_masks.get(&node) {
            return Some(mask);
        }
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
    ) -> Option<computed::FinalStyleRecordID> {
        let record = self.computed_group_sets.assigned_style_record(node)?;
        if !scratch.font_drive.is_pending() {
            // A record the engine derived for the element itself carries its pseudo-elements,
            // and an element standing for its host's pseudo-element is that pseudo-element.
            if self.engine_computed_records_pending.contains_key(&node)
                || self.computed_group_sets.adjustment_facts(node)
                    & bridge::element_adjustment_fact::IS_SHADOW_HOST_PSEUDO_ELEMENT
                    != 0
            {
                return None;
            }
            // Every declaration the pseudo-elements' rules make has to be a winner the engine
            // holds, as it has for any record it derives. What the element's own declarations
            // make is in the record C++ computed.
            if !self.pseudo_winners_are_complete(node) {
                counters.bump(Counter::EngineComputedRecordBailIncompleteWinners);
                return None;
            }
            if !self.engine_pseudo_inputs_available(node, Some(record), counters) {
                return None;
            }
            // The default marker's tabular numerals are not a font the engine resolves yet: a
            // list item's marker would be derived only to be discarded at its font.
            let is_list_item = self
                .computed_group_sets
                .style_record_view(record.raw())
                .and_then(|view| unsafe { view.longhand_table.as_ref() })
                .is_none_or(|table| table.display_is_list_item());
            if (old_is_list_item || is_list_item) && self.marker_declares_unresolvable_numerals(node) {
                counters.bump(Counter::EngineComputedRecordBailFontPhase);
                return None;
            }
        }
        let generation = self.winner_groups.generation();
        if self
            .settle_engine_pseudo_records(
                node,
                None,
                Some(old_is_list_item),
                record,
                generation,
                scratch,
                counters,
            )
            .is_none()
        {
            if scratch.font_drive.request.is_none() {
                // The element's record is C++'s and stays; only what was settled beside it goes.
                for pending in self.engine_computed_records_pending.remove(&node).into_iter().flatten() {
                    let derived = pending.new_style_record;
                    self.revert_engine_computed_pseudo_record(&pending, counters);
                    scratch.pseudo_cohorts.retain(|_, record| *record != derived);
                    self.engine_pseudo_record_cache.retain(|_, record| *record != derived);
                }
                scratch.pseudo_deltas.clear();
                self.settle_computed_memory();
            }
            return None;
        }
        Some(record)
    }

    fn marker_declares_unresolvable_numerals(&self, node: StyleNodeID) -> bool {
        use crate::css::property_metadata::property_id::FONT_VARIANT_NUMERIC;
        self.current_winner_groups()
            .pseudo_states(node)
            .find(|(pseudo, ..)| pseudo.kind.0 == u16::from(pseudo_kind::MARKER))
            .and_then(|(_, _, state, _)| self.winner_groups.winner_in_state(state, FONT_VARIANT_NUMERIC))
            .and_then(|winner| self.winner_groups.resolved_winner(winner))
            .is_some_and(|winner| {
                !matches!(self.specified_values.value(winner.key.value),
                    Lookup::Known(StyleValueData::Keyword { keyword }) if *keyword == crate::css::style_compute::keyword::NORMAL)
            })
    }

    /// Whether every rule the node's answer matches for a pseudo-element declares only what the
    /// winner columns hold, and custom properties, which the engine resolves into the
    /// pseudo-element's own environment, with no container query deciding it.
    fn pseudo_winners_are_complete(&self, node: StyleNodeID) -> bool {
        let rule_is_complete = |rule: RuleID| {
            !self.program.rule_is_gated_by_container_query(rule)
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
            let Some(request) = scratch.font_drive.request.take() else {
                break record;
            };
            suspended_memory.resize_required_to(&mut self.memory, scratch.font_drive.capacity_bytes());
            self.refill_font_requests(vec![(Some(node), request)], counters);
        };
        let mut settled = RetriedEngineRecord::default();
        let Some(record) = record else {
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
