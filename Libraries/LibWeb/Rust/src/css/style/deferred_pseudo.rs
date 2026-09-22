/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

impl StyleEngineState {
    /// Start a new observability interval for one otherwise deferred pseudo-element kind. The
    /// previous interval's nodes receive an exact reaction so their now-unobservable records are
    /// removed, rather than making every latent rule match in the document observable.
    pub fn set_pseudo_element_style_deferred(&mut self, kind: tree::PseudoElementKind, deferred: bool) {
        let previously_observable = std::mem::take(&mut self.retained.deferred_pseudo_element_observable_nodes);
        for node in previously_observable {
            self.record_derived_element_style_input(
                node,
                transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED,
                0,
            );
        }
        self.retained.deferred_pseudo_element = deferred.then_some(kind);
        self.retained.latent_deferred_pseudo_element = Some(kind);
    }

    /// Make a node part of the current observability interval. The document records its
    /// pseudo-element input through the ordinary ordered input path immediately afterwards.
    pub fn make_deferred_pseudo_element_style_observable(&mut self, node: StyleNodeID) {
        match self
            .retained
            .deferred_pseudo_element_observable_nodes
            .binary_search(&node)
        {
            Ok(_) => return,
            Err(index) => self
                .retained
                .deferred_pseudo_element_observable_nodes
                .insert(index, node),
        }
        if let Ok(index) = self
            .host
            .latent_deferred_pseudo_element_style_inputs
            .binary_search_by_key(&InputKey::ElementStyleInput(node), |input| input.key)
        {
            self.host.latent_deferred_pseudo_element_style_inputs.remove(index);
        }
        self.settle_deferred_element_style_input_memory();
    }

    /// Release ordinary deferred reactions into this transaction while retaining pseudo-only
    /// work for nodes outside the current observability interval.
    pub(super) fn flush_deferred_element_style_inputs(&mut self, counters: &mut Counters) {
        for input in std::mem::take(&mut self.host.deferred_element_style_inputs) {
            let InputValue::ElementStyleInput {
                reaction,
                inherited_style_groups,
            } = input.new
            else {
                unreachable!();
            };
            let Some(node) = input.key.style_node() else {
                unreachable!();
            };
            let is_external = self.host.externally_recorded_style_input_nodes.contains(&node);
            let pseudo_is_deferred = self.retained.latent_pseudo_element_reaction_is_deferred_for(node);
            if !is_external
                && pseudo_is_deferred
                && reaction & transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED != 0
            {
                let key = InputKey::ElementStyleInput(node);
                let latent = NormalizedInput {
                    key,
                    old: InputValue::ElementStyleInput {
                        reaction: 0,
                        inherited_style_groups: 0,
                    },
                    new: InputValue::ElementStyleInput {
                        reaction: transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED,
                        inherited_style_groups: 0,
                    },
                };
                if let Err(index) = self
                    .host
                    .latent_deferred_pseudo_element_style_inputs
                    .binary_search_by_key(&key, |input| input.key)
                {
                    self.host
                        .latent_deferred_pseudo_element_style_inputs
                        .insert(index, latent);
                }
                let mut remaining_reaction = reaction & !transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED;
                if remaining_reaction == transaction::STYLE_REACTION_RECOMPUTE_STYLE {
                    remaining_reaction = 0;
                }
                if remaining_reaction != 0 {
                    self.record_input(
                        input.key,
                        input.old,
                        InputValue::ElementStyleInput {
                            reaction: remaining_reaction,
                            inherited_style_groups,
                        },
                        counters,
                    );
                }
            } else {
                self.record_input(input.key, input.old, input.new, counters);
            }
        }
        self.settle_deferred_element_style_input_memory();
    }

    pub(super) fn record_deferred_pseudo_element_style_input(&mut self, node: StyleNodeID) {
        if self.retained.latent_pseudo_element_reaction_is_deferred_for(node) {
            let key = InputKey::ElementStyleInput(node);
            let latent = NormalizedInput {
                key,
                old: InputValue::ElementStyleInput {
                    reaction: 0,
                    inherited_style_groups: 0,
                },
                new: InputValue::ElementStyleInput {
                    reaction: transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED,
                    inherited_style_groups: 0,
                },
            };
            if let Err(index) = self
                .host
                .latent_deferred_pseudo_element_style_inputs
                .binary_search_by_key(&key, |input| input.key)
            {
                self.host
                    .latent_deferred_pseudo_element_style_inputs
                    .insert(index, latent);
            }
            self.settle_deferred_element_style_input_memory();
            return;
        }
        self.record_derived_element_style_input(node, transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED, 0);
    }

    pub(super) fn settle_deferred_element_style_input_memory(&mut self) {
        let bytes = ((self.host.deferred_element_style_inputs.capacity()
            + self.host.latent_deferred_pseudo_element_style_inputs.capacity())
            * size_of::<NormalizedInput>()) as u64;
        self.host
            .deferred_element_style_input_memory
            .resize_required_to(&mut self.retained.memory, bytes);
    }
}

impl RetainedState {
    fn latent_pseudo_element_reaction_is_deferred_for(&self, node: StyleNodeID) -> bool {
        self.latent_deferred_pseudo_element.is_some()
            && self
                .deferred_pseudo_element_observable_nodes
                .binary_search(&node)
                .is_err()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_pseudo_observability_is_scoped_to_explicit_nodes() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut raw_nodes = [0; 2];
        engine.allocate_style_nodes(&mut raw_nodes);
        let [first, second] = raw_nodes.map(|node| StyleNodeID::from_raw(node).unwrap());
        let selection = tree::PseudoElementKind(6);

        engine.set_pseudo_element_style_deferred(selection, false);
        assert_eq!(engine.deferred_pseudo_element, None);
        assert!(engine.latent_pseudo_element_reaction_is_deferred_for(first));
        assert!(engine.latent_pseudo_element_reaction_is_deferred_for(second));

        engine.make_deferred_pseudo_element_style_observable(first);
        assert!(!engine.latent_pseudo_element_reaction_is_deferred_for(first));
        assert!(engine.latent_pseudo_element_reaction_is_deferred_for(second));
        assert!(engine.host.deferred_element_style_inputs.is_empty());

        engine.record_element_style_input(first, transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED, 0);
        assert_eq!(engine.host.deferred_element_style_inputs.len(), 1);
        let InputValue::ElementStyleInput { reaction, .. } = engine.host.deferred_element_style_inputs[0].new else {
            unreachable!();
        };
        assert_eq!(reaction, transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED);

        engine.record_deferred_pseudo_element_style_input(second);
        assert_eq!(engine.host.latent_deferred_pseudo_element_style_inputs.len(), 1);
        assert_eq!(engine.host.deferred_element_style_inputs.len(), 1);

        engine.set_pseudo_element_style_deferred(selection, true);
        assert_eq!(engine.deferred_pseudo_element, Some(selection));
    }
}
