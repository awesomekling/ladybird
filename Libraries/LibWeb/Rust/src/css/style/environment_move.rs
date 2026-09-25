/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Moving the custom-property environments below an element whose own environment moved.

use super::computed::{ComputedStyleTarget, FinalStyleRecordID};
use super::inputs::HeldCustomPropertyEnvironment;
use super::transaction::STYLE_REACTION_RECOMPUTE_STYLE;
use super::{StyleEngineState, StyleNodeID, custom_property_environments};

/// An element a style pass moved to its parent's moved custom-property environment as it settled a
/// row above it. The pass republished the element's record over the moved environment; the element
/// takes the environment itself once the host installs that record, as the host owns what it holds.
pub(crate) struct EnvironmentMoveInFlight {
    /// The element it inherits the moved environment from.
    parent: StyleNodeID,
    /// The environment the element takes.
    environment: u64,
    /// The record the host held for the element before the pass moved it, which a discarded pass
    /// puts it back on.
    held_style_record: u64,
    /// The record the pass republished over the moved environment.
    style_record: u64,
    /// The moves an ancestor made earlier in the pass, which this one took over: the parent, the
    /// environment and the record of each, oldest first.
    earlier: Vec<(StyleNodeID, u64, u64)>,
}

/// What a style pass's walk below a settled row whose environment moved leaves for the pass.
#[derive(Default)]
pub(super) struct EnvironmentMoveInPass {
    /// The elements whose records the walk republished: the element, the record the host holds, and
    /// the record over the moved environment, which the host installs as a row of its own.
    pub(super) republished: Vec<(StyleNodeID, u64, u64)>,
    /// The elements whose style reads the moved environment, which compute again in the pass.
    pub(super) recompute: Vec<StyleNodeID>,
}

fn is_engine_environment(environment: u64) -> bool {
    environment == 0 || environment & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT != 0
}

impl StyleEngineState {
    /// The elements that inherit from `parent` in the flat tree, in its order: its light children
    /// no slot takes, its shadow root's children, and the elements assigned to it as a slot.
    fn flat_tree_element_children(&self, parent: StyleNodeID) -> Vec<StyleNodeID> {
        let tree = &self.retained.tree;
        let mut children = Vec::new();
        let mut next = tree.first_element_child(parent);
        while let Some(child) = next {
            next = tree.next_element_sibling(child);
            if tree.assigned_slot_of(child).is_none() {
                children.push(child);
            }
        }
        let mut next = tree
            .shadow_root_of(parent)
            .and_then(|root| tree.first_element_child(root));
        while let Some(child) = next {
            next = tree.next_element_sibling(child);
            children.push(child);
        }
        if self.retained.facts.is_slot(parent) {
            children.extend(
                tree.assigned_nodes_of(parent)
                    .iter()
                    .copied()
                    .filter(|node| !node.is_text()),
            );
        }
        children
    }

    /// The environment a child inherits from an element holding `environment`: without the
    /// custom properties a registration keeps from inheriting.
    fn inheritable_environment(&mut self, environment: u64) -> u64 {
        if environment == 0 || self.retained.custom_property_environments.store(environment).is_none() {
            return environment;
        }
        let inputs = self.retained.document_style_computation_inputs;
        self.retained
            .inheritable_custom_property_environment(environment, &inputs)
    }

    /// Whether a moved environment computes the element again rather than handing it the moved
    /// one: its style reads a custom property through `var()`, or the environment another way.
    fn environment_move_needs_recompute(&mut self, node: StyleNodeID) -> bool {
        self.retained.element_recomputes_on_environment_move(node)
            || self.retained.node_style_reads_custom_properties(node)
    }

    /// What an element holds for `environment`, taken from its parent: `None` for the empty one,
    /// and nothing at all when it is one the host made and the parent does not hold its object.
    fn held_environment_for(
        &self,
        environment: u64,
        parent: StyleNodeID,
    ) -> Option<Option<HeldCustomPropertyEnvironment>> {
        if environment == 0 {
            return Some(None);
        }
        if environment & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT != 0 {
            return Some(Some(HeldCustomPropertyEnvironment {
                animation_base: None,
                identity: environment,
                is_animation_overlay: false,
                declares: self.environment_declares(environment),
                data: None,
            }));
        }
        let parent_held = self
            .retained
            .element_custom_property_data
            .get(&parent)
            .and_then(Option::as_ref)
            .filter(|held| held.identity == environment && !held.is_animation_overlay)?;
        Some(Some(HeldCustomPropertyEnvironment {
            animation_base: None,
            identity: environment,
            is_animation_overlay: false,
            declares: parent_held.declares,
            data: Some(parent_held.data.as_ref()?.share()),
        }))
    }

    /// Whether an engine-resolved environment declares custom properties of its own.
    fn environment_declares(&self, environment: u64) -> bool {
        self.retained
            .custom_property_environments
            .engine_environment(environment)
            .is_some_and(|(store, _)| {
                // SAFETY: The table keeps the store of every environment it names.
                !unsafe { &*store.cast::<crate::css::custom_properties::CustomPropertyStore>() }
                    .declared_names
                    .is_empty()
            })
    }

    /// The element's pseudo-elements held the environment it moves from, or what inherits from
    /// it; they take the moved one, and an element whose pseudo-element resolved its own computes
    /// again.
    ///
    /// Returns whether a pseudo-element resolved an environment of its own, for the element to
    /// compute again.
    fn move_pseudo_element_environments(
        &mut self,
        element: StyleNodeID,
        existing: u64,
        moved: Option<&HeldCustomPropertyEnvironment>,
    ) -> bool {
        let kinds: Vec<u8> = self
            .retained
            .pseudo_element_custom_property_data
            .keys()
            .filter(|(node, _)| *node == element)
            .map(|(_, kind)| *kind)
            .collect();
        if kinds.is_empty() {
            return false;
        }
        let mut needs_recompute = false;
        let existing_inheritable = self.inheritable_environment(existing);
        let moved_identity = moved.map_or(0, |held| held.identity);
        let moved_inheritable = self.inheritable_environment(moved_identity);
        // A pseudo-element that takes the moved environment inherits it: it declares none of its own.
        for kind in kinds {
            let held = self.retained.pseudo_element_custom_property_data[&(element, kind)].identity;
            let replacement = if held == existing {
                moved.map(|moved| HeldCustomPropertyEnvironment {
                    animation_base: None,
                    identity: moved.identity,
                    is_animation_overlay: false,
                    declares: false,
                    data: moved
                        .data
                        .as_ref()
                        .map(super::inputs::RetainedCustomPropertyData::share),
                })
            } else if held == existing_inheritable && moved_inheritable == moved_identity {
                moved.map(|moved| HeldCustomPropertyEnvironment {
                    animation_base: None,
                    identity: moved.identity,
                    is_animation_overlay: false,
                    declares: false,
                    data: moved
                        .data
                        .as_ref()
                        .map(super::inputs::RetainedCustomPropertyData::share),
                })
            } else if held == existing_inheritable
                && moved_inheritable & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT != 0
            {
                Some(HeldCustomPropertyEnvironment {
                    animation_base: None,
                    identity: moved_inheritable,
                    is_animation_overlay: false,
                    declares: false,
                    data: None,
                })
            } else {
                needs_recompute = true;
                continue;
            };
            match replacement {
                Some(replacement) => {
                    let retired = self
                        .retained
                        .pseudo_element_custom_property_data
                        .insert((element, kind), replacement);
                    self.host
                        .retired_custom_property_data
                        .extend(retired.and_then(|held| held.data));
                }
                None => {
                    let retired = self
                        .retained
                        .pseudo_element_custom_property_data
                        .remove(&(element, kind));
                    self.host
                        .retired_custom_property_data
                        .extend(retired.and_then(|held| held.data));
                }
            }
        }
        needs_recompute
    }
}

impl StyleEngineState {
    /// The record a style pass reads as the one the host holds for `node`: the host's own, or the
    /// one an environment move earlier in the pass republished, which the host installs before any
    /// row below it.
    pub(super) fn held_style_record_in_pass(&self, node: StyleNodeID) -> u64 {
        match self.host.environment_moves_in_flight.get(&node) {
            Some(environment_move) => environment_move.style_record,
            None => self.host.held_style_records.get(&node).copied().unwrap_or(0),
        }
    }

    /// The environment an element holds as a style pass reads it, whether it declares custom
    /// properties of its own, and whether it is an animation overlay.
    fn held_environment_in_pass(&self, element: StyleNodeID) -> (u64, bool, bool) {
        if let Some(environment_move) = self.host.environment_moves_in_flight.get(&element) {
            return (
                environment_move.environment,
                self.environment_declares(environment_move.environment),
                false,
            );
        }
        self.retained
            .element_custom_property_data
            .get(&element)
            .and_then(Option::as_ref)
            .map_or((0, false, false), |held| {
                (held.identity, held.declares, held.is_animation_overlay)
            })
    }

    /// Whether an element holds the environment its parent hands down, as a style pass reads what
    /// both hold: a move of the parent's environment gave it the moved one. The host walks below a
    /// parent holding an animation overlay, and a row whose move the pass did not make reports it,
    /// leaving the element on the environment it held before.
    pub(super) fn holds_parent_environment_in_pass(&mut self, element: StyleNodeID) -> bool {
        if self.host.environment_moves_in_flight.contains_key(&element) {
            return true;
        }
        let Some(parent) = self.retained.tree.flat_tree_parent(element) else {
            return false;
        };
        let (parent_environment, _, parent_is_animation_overlay) = self.held_environment_in_pass(parent);
        let (environment, declares, is_animation_overlay) = self.held_environment_in_pass(element);
        !parent_is_animation_overlay
            && !is_animation_overlay
            && !declares
            && environment == self.inheritable_environment(parent_environment)
    }

    /// A row the pass settled moves the element's custom-property environment to `new_base`: the
    /// pass moves the environments below it here, as the host's walk would once it installs the
    /// row, and leaves what the host holds to the host. Whether the pass moved them: the host walks
    /// below an element holding an animation overlay, or moving to an environment the host made.
    pub(super) fn move_custom_property_environment_in_pass(
        &mut self,
        origin: StyleNodeID,
        new_base: Option<u64>,
        moved: &mut EnvironmentMoveInPass,
    ) -> bool {
        let Some(new_base) = new_base.filter(|&environment| is_engine_environment(environment)) else {
            return false;
        };
        let (old_base, _, is_animation_overlay) = self.held_environment_in_pass(origin);
        if is_animation_overlay {
            return false;
        }
        if old_base != new_base {
            self.move_custom_property_environments_in_pass_below(origin, old_base, new_base, moved);
        }
        true
    }

    fn move_custom_property_environments_in_pass_below(
        &mut self,
        parent: StyleNodeID,
        old_parent_base: u64,
        new_parent_base: u64,
        moved: &mut EnvironmentMoveInPass,
    ) {
        let children = self.flat_tree_element_children(parent);
        if children.is_empty() {
            return;
        }
        let old_parent_inheritable = self.inheritable_environment(old_parent_base);
        let new_parent_inheritable = self.inheritable_environment(new_parent_base);
        for child in children {
            self.move_custom_property_environment_in_pass_of(
                child,
                parent,
                old_parent_inheritable,
                new_parent_inheritable,
                moved,
            );
        }
    }

    /// Move one element below a row whose environment moved: the element either computes again in
    /// the pass or takes the moved environment in a record the pass republishes.
    fn move_custom_property_environment_in_pass_of(
        &mut self,
        element: StyleNodeID,
        parent: StyleNodeID,
        old_parent_inheritable: u64,
        new_parent_inheritable: u64,
        moved: &mut EnvironmentMoveInPass,
    ) {
        // An unstyled subtree materializes against whatever it inherits then.
        let held_record = self.held_style_record_in_pass(element);
        if held_record == 0 {
            return;
        }
        // An element the pass computed a record for already did so over the moved environment.
        let assigned_record = self
            .retained
            .computed_group_sets
            .assigned_final_style_record(ComputedStyleTarget::new(element, u8::MAX))
            .map_or(0, |record| record.raw());
        if assigned_record != held_record {
            return;
        }
        let (existing, existing_declares, is_animation_overlay) = self.held_environment_in_pass(element);
        let holds_inherited_environment = existing == old_parent_inheritable
            || (!existing_declares && !self.retained.node_declares_custom_properties(element));
        // An element whose animations sampled custom properties, whose own declarations resolve
        // over the environment it inherits, or whose style reads the environment computes again.
        if is_animation_overlay || !holds_inherited_environment || self.environment_move_needs_recompute(element) {
            moved.recompute.push(element);
            return;
        }
        if new_parent_inheritable == existing {
            return;
        }
        // The element takes an environment the engine resolved, or none. A pseudo-element that
        // resolved an environment of its own computes the element again.
        if !is_engine_environment(new_parent_inheritable)
            || self.pseudo_environment_move_needs_recompute(element, existing, new_parent_inheritable)
        {
            moved.recompute.push(element);
            return;
        }
        let Some(style_record) = self
            .retained
            .republish_record_environment(element, new_parent_inheritable)
        else {
            moved.recompute.push(element);
            return;
        };
        let (original_held_record, earlier) = match self.host.environment_moves_in_flight.remove(&element) {
            Some(earlier_move) => {
                let mut earlier = earlier_move.earlier;
                earlier.push((earlier_move.parent, earlier_move.environment, earlier_move.style_record));
                (earlier_move.held_style_record, earlier)
            }
            None => (held_record, Vec::new()),
        };
        self.host.environment_moves_in_flight.insert(
            element,
            EnvironmentMoveInFlight {
                parent,
                environment: new_parent_inheritable,
                held_style_record: original_held_record,
                style_record,
                earlier,
            },
        );
        moved.republished.push((element, held_record, style_record));
        self.move_custom_property_environments_in_pass_below(element, existing, new_parent_inheritable, moved);
    }

    /// Whether moving the element from `existing` to `moved_identity` leaves one of its
    /// pseudo-elements on an environment it resolved itself (see `move_pseudo_element_environments`).
    fn pseudo_environment_move_needs_recompute(
        &mut self,
        element: StyleNodeID,
        existing: u64,
        moved_identity: u64,
    ) -> bool {
        let held: Vec<u64> = self
            .retained
            .pseudo_element_custom_property_data
            .iter()
            .filter(|((node, _), _)| *node == element)
            .map(|(_, held)| held.identity)
            .collect();
        if held.is_empty() {
            return false;
        }
        let existing_inheritable = self.inheritable_environment(existing);
        let moved_inheritable = self.inheritable_environment(moved_identity);
        held.into_iter().any(|held| {
            held != existing
                && !(held == existing_inheritable
                    && (moved_inheritable == moved_identity
                        || moved_inheritable & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT != 0))
        })
    }

    /// The host installs the record a style pass republished for `node` over a moved environment:
    /// the element, and its pseudo-elements, take the moved environment. Returns the record to
    /// install, the last one the pass republished for the element, or 0 once the element took it.
    pub(crate) fn acknowledge_environment_move(&mut self, node: StyleNodeID) -> u64 {
        let Some(environment_move) = self.host.environment_moves_in_flight.remove(&node) else {
            return 0;
        };
        let style_record = environment_move.style_record;
        let existing = self
            .retained
            .element_custom_property_data
            .get(&node)
            .and_then(Option::as_ref)
            .map_or(0, |held| held.identity);
        let Some(moved) = self.held_environment_for(environment_move.environment, environment_move.parent) else {
            self.record_derived_element_style_input(node, STYLE_REACTION_RECOMPUTE_STYLE, 0);
            return style_record;
        };
        if self.move_pseudo_element_environments(node, existing, moved.as_ref()) {
            self.record_derived_element_style_input(node, STYLE_REACTION_RECOMPUTE_STYLE, 0);
        }
        let retired = self.retained.element_custom_property_data.insert(node, moved);
        self.host
            .retired_custom_property_data
            .extend(retired.flatten().and_then(|held| held.data));
        style_record
    }

    /// A row the pass settled past the cut of its wave is driven again in the wave that reaches it,
    /// and the move it made of `node` from `previous` to `republished` goes back with it: the move
    /// the host installs, if any, is the one before it. Otherwise the row, driven again, would read
    /// the move as one the host holds, and the host would never install it.
    pub(super) fn unwind_environment_move(&mut self, node: StyleNodeID, republished: u64, previous: u64) {
        let Some(environment_move) = self.host.environment_moves_in_flight.get_mut(&node) else {
            return;
        };
        if environment_move.style_record != republished {
            return;
        }
        match environment_move.earlier.pop() {
            Some((parent, environment, style_record)) => {
                environment_move.parent = parent;
                environment_move.environment = environment;
                environment_move.style_record = style_record;
            }
            None => {
                self.host.environment_moves_in_flight.remove(&node);
            }
        }
        if let (Some(republished), Some(previous)) = (
            FinalStyleRecordID::from_raw(republished),
            FinalStyleRecordID::from_raw(previous),
        ) {
            self.retained
                .computed_group_sets
                .revert_engine_computed_record(node, republished, previous);
        }
    }

    /// The host gave up on the batch: every element whose republished record it did not install
    /// goes back to the record it held.
    pub(super) fn discard_environment_moves_in_flight(&mut self) {
        for (node, environment_move) in std::mem::take(&mut self.host.environment_moves_in_flight) {
            let (Some(republished), Some(held)) = (
                FinalStyleRecordID::from_raw(environment_move.style_record),
                FinalStyleRecordID::from_raw(environment_move.held_style_record),
            ) else {
                continue;
            };
            self.retained
                .computed_group_sets
                .revert_engine_computed_record(node, republished, held);
        }
    }
}
