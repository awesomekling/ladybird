/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Moving the custom-property environments below an element whose own environment moved.

use super::inputs::HeldCustomPropertyEnvironment;
use super::record_replay::EventKind;
use super::transaction::STYLE_REACTION_RECOMPUTE_STYLE;
use super::{StyleEngineState, StyleNodeID, custom_property_environments};

impl StyleEngineState {
    /// An element's custom properties moved from the environment `old_base` to `new_base`, both
    /// without the animation overlay the element may hold over them. Every styled descendant holds
    /// the environment it inherits by identity, so each takes the moved one here, directly, and only
    /// the descendants whose style reads the environment are recorded to compute again. The walk is
    /// the propagation, in the flat tree the engine would have derived reactions over.
    ///
    /// Returns the records the walk moved to their new environments, for the host to install.
    pub(crate) fn move_custom_property_environment(
        &mut self,
        origin: StyleNodeID,
        old_base: u64,
        new_base: u64,
    ) -> Vec<(StyleNodeID, u64)> {
        let mut moved_records = Vec::new();
        self.move_custom_property_environments_below(origin, old_base, new_base, &mut moved_records);
        moved_records
    }

    fn move_custom_property_environments_below(
        &mut self,
        parent: StyleNodeID,
        old_parent_base: u64,
        new_parent_base: u64,
        moved_records: &mut Vec<(StyleNodeID, u64)>,
    ) {
        let children = self.flat_tree_element_children(parent);
        if children.is_empty() {
            return;
        }
        let old_parent_inheritable = self.inheritable_environment(old_parent_base);
        let new_parent_inheritable = self.inheritable_environment(new_parent_base);
        for child in children {
            self.move_custom_property_environment_of(
                child,
                parent,
                old_parent_inheritable,
                new_parent_inheritable,
                moved_records,
            );
        }
    }

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

    /// Record the element to compute again. Its descendants are this walk's, or that
    /// computation's, to reach.
    fn recompute_after_environment_move(&mut self, node: StyleNodeID) {
        self.record_derived_element_style_input(node, STYLE_REACTION_RECOMPUTE_STYLE, 0);
        self.record_boundary_call(EventKind::RecordDerivedElementStyleInput, |payload| {
            payload.write_u32(node.raw());
            payload.write_u8(STYLE_REACTION_RECOMPUTE_STYLE);
            payload.write_u8(0);
        });
    }

    fn move_custom_property_environment_of(
        &mut self,
        element: StyleNodeID,
        parent: StyleNodeID,
        old_parent_inheritable: u64,
        new_parent_inheritable: u64,
        moved_records: &mut Vec<(StyleNodeID, u64)>,
    ) {
        // An unstyled subtree materializes against whatever it inherits then.
        let Some(&held_record) = self.host.held_style_records.get(&element) else {
            return;
        };
        // An element whose row this batch installs later holds a record the engine computed over
        // the moved environment already. The row installs it, and moves the environment below the
        // element in turn; republishing the element's record here would leave its row's record and
        // the one the element held before it to nobody.
        let assigned_record = self
            .retained
            .computed_group_sets
            .assigned_final_style_record(super::computed::ComputedStyleTarget::new(element, u8::MAX))
            .map_or(0, |record| record.raw());
        if assigned_record != held_record {
            return;
        }
        let (existing, is_animation_overlay, existing_declares) = self
            .retained
            .element_custom_property_data
            .get(&element)
            .and_then(Option::as_ref)
            .map_or((0, false, false), |held| {
                (held.identity, held.is_animation_overlay, held.declares)
            });
        // An element's animations sampled custom properties over what its style resolves to; the
        // computation composes them over the moved environment.
        if is_animation_overlay {
            self.recompute_after_environment_move(element);
            return;
        }
        // An element declaring no custom property of its own holds the environment it inherits,
        // whichever identity it holds it by; its cascade declaring some now is a computation's to
        // find.
        let holds_inherited_environment = existing == old_parent_inheritable
            || (!existing_declares && !self.retained.node_declares_custom_properties(element));
        // The element declares custom properties of its own over the environment it inherits,
        // which its computation resolves over the moved one.
        if !holds_inherited_environment {
            self.recompute_after_environment_move(element);
            return;
        }
        if self.environment_move_needs_recompute(element) {
            self.recompute_after_environment_move(element);
            return;
        }
        if new_parent_inheritable == existing {
            return;
        }
        // The environment the element takes: one the engine resolved, which the host views from its
        // store, or the one its parent holds, whose host object it shares.
        let Some(moved) = self.held_environment_for(new_parent_inheritable, parent) else {
            self.recompute_after_environment_move(element);
            return;
        };
        self.move_pseudo_element_environments(element, existing, moved.as_ref());
        if new_parent_inheritable != 0 {
            self.retained
                .computed_group_sets
                .set_node_custom_property_environment(element, new_parent_inheritable);
        }
        let retired = self.retained.element_custom_property_data.insert(element, moved);
        self.host
            .retired_custom_property_data
            .extend(retired.flatten().and_then(|held| held.data));
        if let Some(record) = self
            .retained
            .republish_record_environment(element, new_parent_inheritable)
        {
            self.record_boundary_call(EventKind::RepublishRecordEnvironment, |payload| {
                payload.write_u32(element.raw());
                payload.write_u64(new_parent_inheritable);
                payload.write_u64(record);
            });
            if record != held_record {
                moved_records.push((element, record));
            }
        }
        self.move_custom_property_environments_below(element, existing, new_parent_inheritable, moved_records);
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
    fn move_pseudo_element_environments(
        &mut self,
        element: StyleNodeID,
        existing: u64,
        moved: Option<&HeldCustomPropertyEnvironment>,
    ) {
        let kinds: Vec<u8> = self
            .retained
            .pseudo_element_custom_property_data
            .keys()
            .filter(|(node, _)| *node == element)
            .map(|(_, kind)| *kind)
            .collect();
        if kinds.is_empty() {
            return;
        }
        let existing_inheritable = self.inheritable_environment(existing);
        let moved_identity = moved.map_or(0, |held| held.identity);
        let moved_inheritable = self.inheritable_environment(moved_identity);
        for kind in kinds {
            let held = self.retained.pseudo_element_custom_property_data[&(element, kind)].identity;
            let replacement = if held == existing {
                moved.map(|moved| HeldCustomPropertyEnvironment {
                    identity: moved.identity,
                    is_animation_overlay: false,
                    declares: moved.declares,
                    data: moved
                        .data
                        .as_ref()
                        .map(super::inputs::RetainedCustomPropertyData::share),
                })
            } else if held == existing_inheritable && moved_inheritable == moved_identity {
                moved.map(|moved| HeldCustomPropertyEnvironment {
                    identity: moved.identity,
                    is_animation_overlay: false,
                    declares: moved.declares,
                    data: moved
                        .data
                        .as_ref()
                        .map(super::inputs::RetainedCustomPropertyData::share),
                })
            } else if held == existing_inheritable
                && moved_inheritable & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT != 0
            {
                Some(HeldCustomPropertyEnvironment {
                    identity: moved_inheritable,
                    is_animation_overlay: false,
                    declares: self.environment_declares(moved_inheritable),
                    data: None,
                })
            } else {
                self.recompute_after_environment_move(element);
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
    }
}
