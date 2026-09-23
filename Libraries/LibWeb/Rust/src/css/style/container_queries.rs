/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Container queries, evaluated over the retained container facts: the containers' published
//! container inputs, the previous layout's boxes and the containers' style records. The host's
//! cascade and the engine's publication ask the same question here.

use std::ffi::c_void;

use super::bridge::FfiContainerEffectKind;
use super::tree::StyleNodeID;
use super::*;

/// What a rule's container conditions say for one subject: whether they hold, what kinds of
/// container query they depend on, and the effects the evaluation leaves for the host to record.
#[derive(Clone, Default)]
pub(crate) struct ContainerVerdict {
    pub(crate) matches: bool,
    pub(crate) depends_on_size: bool,
    pub(crate) depends_on_style: bool,
    pub(crate) effects: Vec<(u32, FfiContainerEffectKind, Vec<u16>)>,
}

struct RetainedContainerStyleContext<'a> {
    store: *const c_void,
    registry: &'a crate::css::custom_properties::CustomPropertyRegistry,
    length: &'a crate::css::style_compute::FfiLengthResolutionContext,
    dependencies: &'a mut crate::css::cascaded_properties::StyleQueryDependencies,
    tree_counting: (u64, u64),
    color_resolution_input: crate::css::color_resolution::ColorResolutionInput<'a>,
}

unsafe extern "C" fn retained_container_style_feature(
    context: *mut c_void,
    feature: crate::css::parser::query_parser::FfiContainerStyleFeature,
) -> u8 {
    let context = unsafe { &mut *context.cast::<RetainedContainerStyleContext<'_>>() };
    unsafe {
        crate::css::custom_properties::evaluate_retained_container_style_feature(
            context.store,
            context.registry,
            feature,
            context.length,
            context.dependencies,
            context.tree_counting,
            context.color_resolution_input,
        ) as u8
    }
}

impl RetainedState {
    /// Keep what a row the engine answers read of its containers for the host, which records it
    /// when it installs the element's record, as it does for a row it computes itself.
    pub(crate) fn note_container_effects_for_host(&mut self, node: StyleNodeID, verdict: &ContainerVerdict) {
        let noted = self.container_effects_for_host.entry(node).or_default();
        noted.depends_on_size |= verdict.depends_on_size;
        noted.depends_on_style |= verdict.depends_on_style;
        noted.effects.extend(verdict.effects.iter().cloned());
    }

    pub(crate) fn note_container_unit_effects_for_host(
        &mut self,
        node: StyleNodeID,
        record: computed::FinalStyleRecordID,
        mask: u8,
    ) {
        if mask == 0 {
            return;
        }
        let Some(inputs) = self.document_style_computation_inputs else {
            return;
        };
        let Some(payloads) = self.computed_group_sets.style_record_payloads(record.raw()) else {
            return;
        };
        let inherited_box = unsafe {
            payloads[crate::css::computed_value_types::STYLE_GROUP_INDEX_INHERITED_BOX]
                .cast::<crate::css::computed_values::InheritedBoxValues>()
                .deref()
        };
        let inline_axis_is_horizontal =
            inherited_box.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB;
        let (needs_width, needs_height) =
            crate::css::style_compute::container_relative_axes_needed(mask, inline_axis_is_horizontal);
        let mut verdict = ContainerVerdict {
            depends_on_size: true,
            ..Default::default()
        };
        for basis in [
            needs_width.then(|| self.container_unit_basis(node, true, inputs.viewport_width)),
            needs_height.then(|| self.container_unit_basis(node, false, inputs.viewport_height)),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(container) = basis.container {
                verdict
                    .effects
                    .push((container.raw(), FfiContainerEffectKind::SizeContainerUsage, Vec::new()));
                if basis.container_has_no_box {
                    verdict.effects.push((
                        container.raw(),
                        FfiContainerEffectKind::NeedsEvaluationAfterLayout,
                        Vec::new(),
                    ));
                }
            }
            if basis.depends_on_viewport_metrics {
                verdict.effects.push((
                    node.raw(),
                    FfiContainerEffectKind::SubjectViewportDependency,
                    Vec::new(),
                ));
            }
        }
        self.note_container_effects_for_host(node, &verdict);
    }

    /// Whether the winners published for a node hold a rule's container conditions: an element's
    /// winners hold a gated rule where its conditions held when they were published, and the record
    /// loop checks that they still do, including the originating element's pseudo-elements.
    pub(crate) fn container_gate_is_held(&self, node: Option<StyleNodeID>, rule: RuleID, _pseudo: bool) -> bool {
        !self.program.rule_is_gated_by_container_query(rule)
            || node.is_some_and(|node| !self.container_gates_unheld.contains(&node))
    }

    /// Decide, as a node's winners are published, whether its gated rules can be: their
    /// conditions read the containers above it, which are final only when no ancestor's answer
    /// moves in the same transaction.
    pub(super) fn note_container_gates_for_publication(&mut self, node: StyleNodeID, effects: &AnswerEffects) {
        let mut ancestor = self.tree.flat_tree_parent(node);
        let mut moving = false;
        while let Some(current) = ancestor {
            if effects.lookup(current).is_some() {
                moving = true;
                break;
            }
            ancestor = self.tree.flat_tree_parent(current);
        }
        if moving {
            self.container_gates_unheld.insert(node);
        } else {
            self.container_gates_unheld.remove(&node);
        }
    }

    /// A node whose gated rules decide differently over the containers as they stand now than
    /// when its winners were published needs new winners from the retained selector answer.
    /// If the exact answer was evicted, ordinary answer completion must reconstruct it.
    pub(super) fn refresh_winners_whose_container_verdicts_moved(
        &mut self,
        rule_program_is_changing: bool,
        counters: &mut Counters,
    ) {
        if self.published_container_verdicts.is_empty() {
            return;
        }
        let moved: Vec<StyleNodeID> = self
            .published_container_verdicts
            .keys()
            .copied()
            .filter(|&node| self.container_verdicts_moved(node))
            .collect();
        for node in moved {
            // The old exact answer can name a rule this transaction removes or replaces. Let
            // routing under the new program publish its winners instead of reviving that rule.
            if rule_program_is_changing || self.republish_winners_from_answer(node, counters).is_none() {
                self.published_container_verdicts.remove(&node);
                self.winner_groups.remove(node);
            }
        }
    }

    pub(super) fn container_verdicts_moved(&self, node: StyleNodeID) -> bool {
        self.published_container_verdicts.get(&node).is_some_and(|verdicts| {
            verdicts.iter().any(|&(rule, pseudo, held)| {
                self.rule_container_verdict(rule, node.raw(), pseudo)
                    .is_none_or(|verdict| verdict.matches != held)
            })
        })
    }

    /// Publish new winners from the retained selector answer, matching from published facts when
    /// that answer has been evicted or its rule dispatch can no longer materialize it.
    pub(super) fn republish_winners_from_answer(&mut self, node: StyleNodeID, counters: &mut Counters) -> Option<bool> {
        let retained_matches = self.current_answer_identity(node).and_then(|identity| {
            let answer = self.match_answers.answer(identity).cloned()?;
            for entry in answer.iter() {
                self.prepare_scope_program(entry.tree_scope);
            }
            let mut matches = Vec::new();
            self.append_catalog_answer(identity, node, None, &mut matches)?;
            Some(matches)
        });
        let matches = match retained_matches {
            Some(matches) => matches,
            None => self.match_element_for_cascade(node, counters).ok()?,
        };
        self.container_gates_unheld.remove(&node);
        let complete = self.cascade_winner_inventory_is_complete(&matches, Some(node));
        let complete_but_for_custom_properties = self.element_declarations_are_complete_but_for_custom_properties(node)
            && matches.iter().all(|entry| {
                self.match_is_complete_but_for_custom_properties(
                    node,
                    entry.rule,
                    entry.tree_scope,
                    entry.pseudo_element.is_some(),
                )
            });
        let mut effects = AnswerEffects::default();
        let compact = self.matches_for_cascade(&mut effects, matches, true, Some(node), counters);
        self.remember_cascade_input_with_effects(&mut effects, node, &compact, counters);
        self.install_answer_effects(effects);
        self.batch_answers_complete_but_for_custom_properties
            .insert(node, complete_but_for_custom_properties);
        let answer_is_incomplete = !complete && !complete_but_for_custom_properties;
        self.computed_group_sets
            .set_node_answer_incomplete(node, answer_is_incomplete);
        Some(complete)
    }

    /// Re-evaluate pseudo winners after the element winner was compared in this flush. The
    /// retained answer already has the selector result; keep the element row and cascade-input
    /// identity while publishing only the pseudo rows derived from it.
    pub(super) fn republish_pseudo_winners_from_retained_answer(
        &mut self,
        node: StyleNodeID,
        counters: &mut Counters,
    ) -> Option<()> {
        let identity = self.current_answer_identity(node)?;
        let answer = self.match_answers.answer(identity).cloned()?;
        for entry in answer.iter() {
            self.prepare_scope_program(entry.tree_scope);
        }
        let mut matches = Vec::new();
        self.append_catalog_answer(identity, node, None, &mut matches)?;
        let mut effects = AnswerEffects::default();
        self.matches_for_cascade(&mut effects, matches, true, Some(node), counters);
        effects.winners.discard_element_row(&mut self.winner_groups, node);
        effects
            .winners
            .preserve_equal_pseudo_states(&mut self.winner_groups, node);
        self.install_answer_effects(effects);
        Some(())
    }

    /// Whether a rule decides for the node as far as its container conditions go: an ungated rule
    /// always does, a gated one where they held when the node's winners were published.
    pub(crate) fn published_container_verdict_holds(&self, node: StyleNodeID, rule: RuleID, pseudo: bool) -> bool {
        !self.program.rule_is_gated_by_container_query(rule)
            || self.published_container_verdicts.get(&node).is_some_and(|verdicts| {
                verdicts
                    .iter()
                    .any(|&(gated, target_is_pseudo, held)| gated == rule && target_is_pseudo == pseudo && held)
            })
    }

    pub(super) fn publish_container_verdicts(&mut self, node: StyleNodeID, verdicts: Vec<(RuleID, bool, bool)>) {
        if verdicts.is_empty() {
            self.published_container_verdicts.remove(&node);
        } else {
            self.published_container_verdicts.insert(node, verdicts);
        }
    }

    /// Whether every gated rule of a node's winners decides now as it did when they were published,
    /// over its containers as its settled ancestors left them. What the evaluations read of the
    /// containers is kept for the host, which records it with the node's record.
    pub(super) fn container_verdicts_stand(&mut self, node: StyleNodeID) -> bool {
        let Some(published) = self.published_container_verdicts.get(&node).cloned() else {
            return true;
        };
        let mut verdicts = Vec::with_capacity(published.len());
        for (rule, pseudo, held) in published {
            let Some(verdict) = self.rule_container_verdict(rule, node.raw(), pseudo) else {
                return false;
            };
            if verdict.matches != held {
                return false;
            }
            verdicts.push(verdict);
        }
        for verdict in &verdicts {
            self.note_container_effects_for_host(node, verdict);
        }
        true
    }

    /// Whether a flat-tree ancestor of the node was declined in this batch: the record the host
    /// computes for it is installed after the batch, and so are the container inputs it publishes.
    pub(super) fn container_ancestor_is_unsettled(
        &self,
        node: StyleNodeID,
        scratch: &super::publication::EngineComputedRecordScratch,
    ) -> bool {
        // Every element is a container a style query can ask about; a size or scroll-state query
        // asks about the containers the element's ancestors declare.
        let asks_about_style = self.container_gates_unheld.contains(&node)
            || self.published_container_verdicts.get(&node).is_some_and(|verdicts| {
                verdicts.iter().any(|&(rule, pseudo, _)| {
                    self.rule_container_verdict(rule, node.raw(), pseudo)
                        .is_none_or(|verdict| verdict.depends_on_style)
                })
            });
        let mut ancestor = self.tree.flat_tree_parent(node);
        while let Some(current) = ancestor {
            if let Some(index) = current.element_index()
                && scratch
                    .derived_child_inputs
                    .get(index as usize)
                    .is_some_and(|row| !row.settled)
                && (asks_about_style || self.may_be_a_query_container(current))
            {
                return true;
            }
            ancestor = self.tree.flat_tree_parent(current);
        }
        false
    }

    /// Whether an element is a size, scroll-state or named container, or its winners may make it
    /// one: an element that is none now and declares no `container-type` is none a size or
    /// scroll-state query asks about.
    fn may_be_a_query_container(&self, node: StyleNodeID) -> bool {
        use crate::css::property_metadata::property_id::CONTAINER_TYPE;
        if self.container_query_inputs(node).is_some_and(|inputs| {
            inputs.is_size_container
                || inputs.is_inline_size_container
                || inputs.is_scroll_state_container
                || !inputs.names.is_empty()
        }) {
            return true;
        }
        match self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        {
            Lookup::Known((_, state)) => self.winner_groups.winner_in_state(state, CONTAINER_TYPE).is_some(),
            _ => true,
        }
    }

    pub(crate) fn take_container_effects_for_host(&mut self, node: StyleNodeID) -> Option<ContainerVerdict> {
        self.container_effects_for_host.remove(&node)
    }

    /// Evaluate a rule's container conditions for a subject. `None` when the rule has no target.
    pub(crate) fn rule_container_verdict(
        &self,
        rule: RuleID,
        subject: u32,
        subject_is_pseudo_element: bool,
    ) -> Option<ContainerVerdict> {
        use crate::css::parser::query_parser::CONTAINER_QUERY_REQUIRES_STYLE;
        let containers = self.native_rules.targets.get(&rule)?.containers().to_vec();
        // Mark all ancestor dependencies, even if an inner condition subsequently fails to match.
        let size = containers.iter().any(|conditions| conditions.contains_size_feature());
        let style = containers.iter().any(|conditions| {
            conditions.conditions.iter().any(|condition| {
                condition.query.as_ref().is_some_and(|query| {
                    let requirements = query.container_requirements();
                    requirements & CONTAINER_QUERY_REQUIRES_STYLE != 0
                })
            })
        });
        let mut effects = Vec::new();
        let matches = containers.iter().all(|conditions| {
            conditions.conditions.iter().any(|condition| {
                let name = condition.name.as_ref().map_or(&[][..], |name| name.units());
                let query = condition
                    .query
                    .as_ref()
                    .map_or(std::ptr::null(), |query| std::sync::Arc::as_ptr(query).cast());
                self.container_condition_matches(subject, subject_is_pseudo_element, query, name, &mut effects)
                    .unwrap_or(false)
            })
        });
        Some(ContainerVerdict {
            matches,
            depends_on_size: size,
            depends_on_style: style,
            effects,
        })
    }

    /// Evaluate a compiled @function declaration's container gates over the same retained
    /// container inputs used by style rules. Missing inputs leave the row with the host.
    pub(super) fn function_container_verdict(
        &self,
        node: StyleNodeID,
        pseudo: bool,
        containers: &[std::sync::Arc<crate::css::container_conditions::ContainerConditionsData>],
    ) -> Option<ContainerVerdict> {
        use crate::css::parser::query_parser::CONTAINER_QUERY_REQUIRES_STYLE;
        let depends_on_size = containers.iter().any(|conditions| conditions.contains_size_feature());
        let depends_on_style = containers.iter().any(|conditions| {
            conditions.conditions.iter().any(|condition| {
                condition
                    .query
                    .as_ref()
                    .is_some_and(|query| query.container_requirements() & CONTAINER_QUERY_REQUIRES_STYLE != 0)
            })
        });
        let mut effects = Vec::new();
        let mut matches = true;
        for conditions in containers {
            let mut group_matches = false;
            for condition in &conditions.conditions {
                let name = condition.name.as_ref().map_or(&[][..], |name| name.units());
                let query = condition
                    .query
                    .as_ref()
                    .map_or(std::ptr::null(), |query| std::sync::Arc::as_ptr(query).cast());
                if self.container_condition_matches(node.raw(), pseudo, query, name, &mut effects)? {
                    group_matches = true;
                    break;
                }
            }
            matches &= group_matches;
        }
        Some(ContainerVerdict {
            matches,
            depends_on_size,
            depends_on_style,
            effects,
        })
    }

    fn container_condition_matches(
        &self,
        subject_raw: u32,
        subject_is_pseudo_element: bool,
        query: *const c_void,
        name: &[u16],
        effects: &mut Vec<(u32, FfiContainerEffectKind, Vec<u16>)>,
    ) -> Option<bool> {
        use crate::css::parser::query_parser::{
            CONTAINER_QUERY_HAS_UNKNOWN_FEATURE, CONTAINER_QUERY_REQUIRES_BLOCK_SIZE, CONTAINER_QUERY_REQUIRES_HEIGHT,
            CONTAINER_QUERY_REQUIRES_INLINE_SIZE, CONTAINER_QUERY_REQUIRES_SCROLL_STATE,
            CONTAINER_QUERY_REQUIRES_WIDTH, FfiContainerFacts, FfiQueryHandle, MatchResult, SCROLL_STATE_SIDE_BOTTOM,
            SCROLL_STATE_SIDE_LEFT, SCROLL_STATE_SIDE_RIGHT, SCROLL_STATE_SIDE_TOP,
        };
        let engine = self;
        let subject = StyleNodeID::from_raw(subject_raw)?;
        let query = unsafe { query.cast::<FfiQueryHandle>().as_ref() };
        let requirements = query.map_or(0, FfiQueryHandle::container_requirements);
        if requirements & CONTAINER_QUERY_HAS_UNKNOWN_FEATURE != 0 {
            return Some(false);
        }
        let mut container = if subject_is_pseudo_element {
            Some(subject)
        } else {
            engine.tree.flat_tree_parent(subject)
        };
        while let Some(candidate) = container {
            container = engine.tree.flat_tree_parent(candidate);
            let Some(inputs) = engine.container_query_inputs(candidate) else {
                // A pseudo-element can ask about its originating element while that element's
                // first style record is still being installed. Keep the scroll dependency even
                // though the container verdict cannot yet be decided: otherwise no scroll-state
                // snapshot is collected to make a later verdict change observable.
                if subject_is_pseudo_element
                    && candidate == subject
                    && requirements & CONTAINER_QUERY_REQUIRES_SCROLL_STATE != 0
                {
                    effects.push((candidate.raw(), FfiContainerEffectKind::SizeContainerUsage, Vec::new()));
                    effects.push((
                        candidate.raw(),
                        FfiContainerEffectKind::ScrollStateContainerUsage,
                        Vec::new(),
                    ));
                    effects.push((
                        candidate.raw(),
                        FfiContainerEffectKind::NeedsEvaluationAfterLayout,
                        Vec::new(),
                    ));
                }
                continue;
            };
            if !name.is_empty() && !inputs.names.iter().any(|candidate| candidate == name) {
                continue;
            }
            let inline_axis_horizontal = inputs.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB;
            let satisfies_width = if inline_axis_horizontal {
                inputs.is_size_container || inputs.is_inline_size_container
            } else {
                inputs.is_size_container
            };
            let satisfies_height = if inline_axis_horizontal {
                inputs.is_size_container
            } else {
                inputs.is_size_container || inputs.is_inline_size_container
            };
            if requirements & CONTAINER_QUERY_REQUIRES_WIDTH != 0 && !satisfies_width
                || requirements & CONTAINER_QUERY_REQUIRES_HEIGHT != 0 && !satisfies_height
                || requirements & CONTAINER_QUERY_REQUIRES_INLINE_SIZE != 0
                    && !(inputs.is_size_container || inputs.is_inline_size_container)
                || requirements & CONTAINER_QUERY_REQUIRES_BLOCK_SIZE != 0 && !inputs.is_size_container
                || requirements & CONTAINER_QUERY_REQUIRES_SCROLL_STATE != 0 && !inputs.is_scroll_state_container
            {
                continue;
            }
            let Some(query) = query else {
                return Some(true);
            };
            let tree_counting = if let Some(parent) = engine.tree.parent(candidate) {
                let mut count = 0_u64;
                let mut index = 0_u64;
                for child in engine.tree.children(parent) {
                    count += 1;
                    if child == candidate {
                        index = count;
                    }
                }
                (count, index)
            } else {
                (1, 1)
            };
            let snapshot = engine.layout_style_snapshots.row(candidate).unwrap_or_default();
            let document = engine.document_style_computation_inputs.as_ref()?;
            let payloads = engine.computed_group_sets.style_record_payloads(inputs.style_record)?;
            let values = crate::css::computed_value_views::ComputedValuesView::new(
                crate::css::host_shared::SharedPayload::as_pointer_slice(payloads),
            );
            let mut resolved_viewport_relative_length = false;
            let width = crate::css::css_pixels::CssPixels::from_raw(snapshot.content_width_raw).to_double();
            let height = crate::css::css_pixels::CssPixels::from_raw(snapshot.content_height_raw).to_double();
            let container_unit_basis = |horizontal: bool| {
                let mut ancestor = engine.tree.flat_tree_parent(candidate);
                while let Some(node) = ancestor {
                    ancestor = engine.tree.flat_tree_parent(node);
                    let Some(ancestor_inputs) = engine.container_query_inputs(node) else {
                        continue;
                    };
                    let ancestor_inline_axis_horizontal =
                        ancestor_inputs.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB;
                    let eligible = if horizontal == ancestor_inline_axis_horizontal {
                        ancestor_inputs.is_size_container || ancestor_inputs.is_inline_size_container
                    } else {
                        ancestor_inputs.is_size_container
                    };
                    if !eligible {
                        continue;
                    }
                    let snapshot = engine.layout_style_snapshots.row(node).unwrap_or_default();
                    if !snapshot.has_committed_box {
                        return (0.0, false, Some(node));
                    }
                    let raw = if horizontal {
                        snapshot.content_width_raw
                    } else {
                        snapshot.content_height_raw
                    };
                    return (
                        crate::css::css_pixels::CssPixels::from_raw(raw).to_double(),
                        false,
                        Some(node),
                    );
                }
                (
                    if horizontal {
                        document.viewport_width
                    } else {
                        document.viewport_height
                    },
                    true,
                    None,
                )
            };
            let (container_width_basis, container_width_basis_depends_on_viewport_metrics, container_width_basis_node) =
                container_unit_basis(true);
            let (
                container_height_basis,
                container_height_basis_depends_on_viewport_metrics,
                container_height_basis_node,
            ) = container_unit_basis(false);
            let length = crate::css::style_compute::FfiLengthResolutionContext {
                viewport_width: document.viewport_width,
                viewport_height: document.viewport_height,
                font_metrics: crate::css::style_compute::FfiFontMetrics {
                    font_size: values.font_size().to_double(),
                    x_height: super::publication::drive_font_metric(values.font_x_height()),
                    cap_height: super::publication::drive_font_metric(values.font_ascent()),
                    zero_advance: super::publication::drive_font_metric(values.font_zero_advance()),
                    line_height: values.line_height().to_double(),
                },
                root_font_metrics: crate::css::style_compute::FfiFontMetrics {
                    font_size: document.root_font_size,
                    x_height: document.root_font_x_height,
                    cap_height: document.root_font_cap_height,
                    zero_advance: document.root_font_zero_advance,
                    line_height: document.root_line_height,
                },
                font_metrics_depend_on_viewport_metrics: false,
                root_font_metrics_depend_on_viewport_metrics: document.root_font_metrics_depend_on_viewport_metrics,
                has_container_width_basis: true,
                has_container_height_basis: true,
                container_width_basis,
                container_height_basis,
                container_width_basis_depends_on_viewport_metrics,
                container_height_basis_depends_on_viewport_metrics,
                subject_inline_axis_is_horizontal: inline_axis_horizontal,
                resolved_viewport_relative_length: &raw mut resolved_viewport_relative_length,
            };
            let opposite = |side: u8| (side + 2) % 4;
            let (block_start_side, mut inline_start_side) = match inputs.writing_mode {
                crate::css::css_enums::writing_mode::HORIZONTAL_TB => (SCROLL_STATE_SIDE_TOP, SCROLL_STATE_SIDE_LEFT),
                crate::css::css_enums::writing_mode::VERTICAL_RL | crate::css::css_enums::writing_mode::SIDEWAYS_RL => {
                    (SCROLL_STATE_SIDE_RIGHT, SCROLL_STATE_SIDE_TOP)
                }
                crate::css::css_enums::writing_mode::VERTICAL_LR => (SCROLL_STATE_SIDE_LEFT, SCROLL_STATE_SIDE_TOP),
                crate::css::css_enums::writing_mode::SIDEWAYS_LR => (SCROLL_STATE_SIDE_LEFT, SCROLL_STATE_SIDE_BOTTOM),
                _ => return None,
            };
            if inputs.direction == crate::css::css_enums::direction::RTL {
                inline_start_side = opposite(inline_start_side);
            }
            let layout_inline_axis_horizontal =
                snapshot.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB;
            let environment = engine
                .computed_group_sets
                .style_record_custom_property_environment(inputs.style_record)
                .unwrap_or_default();
            let store = engine
                .custom_property_environments
                .store(environment)
                .unwrap_or(std::ptr::null());
            let registry = engine.custom_property_registry.as_deref()?;
            let mut dependencies = crate::css::cascaded_properties::StyleQueryDependencies::default();
            let packed_color = values.inherited_text().color;
            let color_resolution_input = crate::css::color_resolution::ColorResolutionInput {
                scheme: Some(values.inherited_ui().color_scheme),
                current_color: Some(crate::css::color_resolution::Rgba {
                    r: (packed_color >> 16) as u8,
                    g: (packed_color >> 8) as u8,
                    b: packed_color as u8,
                    a: (packed_color >> 24) as u8,
                }),
                current_color_value: values.inherited_text().color_style_value.data(),
                length: Some(&length),
                channels: None,
            };
            let mut style_context = RetainedContainerStyleContext {
                store,
                registry,
                length: &length,
                dependencies: &mut dependencies,
                tree_counting,
                color_resolution_input,
            };
            let facts = FfiContainerFacts {
                container_available: true,
                size_available: snapshot.has_committed_box,
                width,
                height,
                inline_axis_horizontal: layout_inline_axis_horizontal,
                length_resolution_context: std::ptr::from_ref(&length).cast(),
                style_context: std::ptr::from_mut(&mut style_context).cast(),
                evaluate_style_feature: retained_container_style_feature,
                scroll_state_available: requirements & CONTAINER_QUERY_REQUIRES_SCROLL_STATE != 0,
                stuck: snapshot.stuck,
                snapped: snapshot.snapped,
                scrollable: snapshot.scrollable,
                scrolled: snapshot.scrolled,
                block_start_side,
                inline_start_side,
            };
            if requirements
                & (CONTAINER_QUERY_REQUIRES_WIDTH
                    | CONTAINER_QUERY_REQUIRES_HEIGHT
                    | CONTAINER_QUERY_REQUIRES_INLINE_SIZE
                    | CONTAINER_QUERY_REQUIRES_BLOCK_SIZE
                    | CONTAINER_QUERY_REQUIRES_SCROLL_STATE)
                != 0
            {
                effects.push((candidate.raw(), FfiContainerEffectKind::SizeContainerUsage, Vec::new()));
            }
            for basis in [container_width_basis_node, container_height_basis_node]
                .into_iter()
                .flatten()
            {
                effects.push((basis.raw(), FfiContainerEffectKind::SizeContainerUsage, Vec::new()));
            }
            if requirements & crate::css::parser::query_parser::CONTAINER_QUERY_REQUIRES_STYLE != 0 {
                effects.push((candidate.raw(), FfiContainerEffectKind::StyleContainerUsage, Vec::new()));
            }
            if requirements & CONTAINER_QUERY_REQUIRES_SCROLL_STATE != 0 {
                effects.push((
                    candidate.raw(),
                    FfiContainerEffectKind::ScrollStateContainerUsage,
                    Vec::new(),
                ));
            }
            if !snapshot.has_committed_box {
                effects.push((
                    candidate.raw(),
                    FfiContainerEffectKind::NeedsEvaluationAfterLayout,
                    Vec::new(),
                ));
            }
            let result = query.evaluate_container_with_tree_counting(&facts, Some(tree_counting)) == MatchResult::True;
            if resolved_viewport_relative_length {
                effects.push((
                    subject_raw,
                    FfiContainerEffectKind::SubjectViewportDependency,
                    Vec::new(),
                ));
            }
            effects.extend(
                dependencies
                    .into_names()
                    .into_iter()
                    .map(|name| (subject_raw, FfiContainerEffectKind::CustomPropertyReference, name)),
            );
            return Some(result);
        }
        Some(false)
    }
}
