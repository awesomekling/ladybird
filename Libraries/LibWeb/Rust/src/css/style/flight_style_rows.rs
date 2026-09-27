/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The rows of a style pass's batch a flight applies to the layout nodes itself, ahead of the host
//! that installs the batch on its elements once the flight is taken back.
//!
//! A flight that goes on from its style pass to the layout rounds applies what the rounds read of
//! the batch: each row's record, bound to the row's layout nodes, and what the row's move damages
//! of layout, paint and the visual contexts. It does so only for a batch whose install leaves the
//! host nothing that reaches them: every row is an element's record the engine computed over the
//! one the element holds, with the whole damage answered, that rebuilds nothing, moves no custom
//! property environment, animates nothing and names no anchor or image, and whose children the
//! engine derived itself. Any other batch is installed by the host before layout, as without the
//! flight.

use super::bridge::{FfiStyleDeltaGap, FfiStyleInvalidationField};
use super::style_invalidation::unpack_invalidation;
use super::{StyleEngine, StyleNodeID, transaction};
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::host_shared::SharedPayload;
use std::collections::HashSet;

/// A row of a style pass's batch a flight applies to the layout nodes of its element.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlightStyleRow {
    pub(crate) style_node: StyleNodeID,
    pub(crate) old_style_record: u64,
    pub(crate) new_style_record: u64,
    /// What the move damages, packed as an `FfiStyleInvalidationField` word.
    pub(crate) damage: u32,
    /// Whether the element is one the viewport takes its overflow, writing mode and direction
    /// from, which only a full layout pass propagates again.
    pub(crate) viewport_propagation_source: bool,
}

/// Why a flight leaves a style pass's batch to the host.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfiFlightStyleDecline {
    /// The pass published nothing for the flight to read.
    NoBatch,
    /// A row is a pseudo-element's, or one the host settles in a way of its own.
    Row,
    /// A row is an element's first style.
    FirstStyle,
    /// A row's damage is the host's to compare.
    Damage,
    /// A row rebuilds the layout tree.
    Rebuild,
    /// A row's move reaches the styles of its descendants, or the children the host derives.
    Descendants,
    /// A row's element animates or owes its animations or transitions something.
    Animation,
    /// A row moved a custom property environment.
    CustomProperties,
    /// A row's records hold images or name anchors.
    Resources,
    /// A row's layout node is one the flight does not style.
    LayoutNode,
    /// A row's layout node holds another record than the one the row moves away from.
    StaleLayoutNode,
}

pub(crate) const FLIGHT_STYLE_DECLINE_COUNT: usize = FfiFlightStyleDecline::StaleLayoutNode as usize + 1;

impl StyleEngine {
    /// The rows of the batch the submitted style pass left that a flight applies to the layout
    /// nodes itself, or why it leaves the batch to the host. `viewport_propagation_sources` names
    /// the elements the viewport propagates from.
    pub(crate) fn rows_a_flight_applies(
        &self,
        viewport_propagation_sources: &[StyleNodeID],
    ) -> Result<Vec<FlightStyleRow>, FfiFlightStyleDecline> {
        let Some((_, output)) = self.host.submitted_style_pass_output.as_ref() else {
            return Err(FfiFlightStyleDecline::NoBatch);
        };
        self.rows_applied_ahead_of_host(output.answers(), viewport_propagation_sources)
    }

    /// The rows of the batch the style transaction the render owner took left, which the owner
    /// applies to the layout nodes itself as the transaction ends, as a flight does for its pass,
    /// or why it leaves the batch to the host.
    pub(crate) fn rows_the_owner_applies(
        &self,
        viewport_propagation_sources: &[StyleNodeID],
    ) -> Result<Vec<FlightStyleRow>, FfiFlightStyleDecline> {
        self.rows_applied_ahead_of_host(
            self.host.ffi_style_transaction_output.answers(),
            viewport_propagation_sources,
        )
    }

    fn rows_applied_ahead_of_host(
        &self,
        answers: &[super::bridge::FfiStyleDelta],
        viewport_propagation_sources: &[StyleNodeID],
    ) -> Result<Vec<FlightStyleRow>, FfiFlightStyleDecline> {
        let animated_nodes: HashSet<StyleNodeID> = self.animated_nodes().collect();
        let mut rows = Vec::with_capacity(answers.len());
        for answer in answers {
            if answer.gap == FfiStyleDeltaGap::SkippedHidden {
                continue;
            }
            if answer.pseudo_kind != u8::MAX || answer.gap != FfiStyleDeltaGap::Computed {
                return Err(FfiFlightStyleDecline::Row);
            }
            let Some(node) = StyleNodeID::from_raw(answer.style_node) else {
                return Err(FfiFlightStyleDecline::Row);
            };
            if answer.reaction
                & (transaction::STYLE_REACTION_ANCESTOR_BECAME_VISIBLE
                    | transaction::STYLE_REACTION_INHERITED_CUSTOM_PROPERTIES
                    | transaction::STYLE_REACTION_RECOMPUTE_DESCENDANT_STYLES)
                != 0
            {
                return Err(FfiFlightStyleDecline::Descendants);
            }
            if answer.old_style_record == 0 || answer.new_style_record == 0 {
                return Err(FfiFlightStyleDecline::FirstStyle);
            }
            // The pass moves what a published row owes its transitions, animations and explicitly
            // inheriting children out of the engine's maps into the row, for the host's install.
            if answer.row_effect_debt != 0 || answer.explicit_inheritance_debt != 0 {
                return Err(FfiFlightStyleDecline::Animation);
            }
            if animated_nodes.contains(&node)
                || self.nodes_owing_a_transition_registration.contains_key(&node)
                || self.nodes_owing_an_animation_sample.contains(&node)
                || self.rows_sampled_in_pass.contains_key(&node)
                || self.pseudo_settles_owed.contains_key(&node)
                || self
                    .nodes_owing_animation_definitions
                    .keys()
                    .any(|(owner, _)| *owner == node)
            {
                return Err(FfiFlightStyleDecline::Animation);
            }
            let damage = if answer.old_style_record == answer.new_style_record {
                0
            } else {
                let answered =
                    FfiStyleInvalidationField::EngineComputed as u32 | FfiStyleInvalidationField::DamageIsTotal as u32;
                if answer.record_damage & answered != answered {
                    return Err(FfiFlightStyleDecline::Damage);
                }
                let invalidation = unpack_invalidation(answer.record_damage);
                if invalidation.rebuilds_layout_tree() {
                    return Err(FfiFlightStyleDecline::Rebuild);
                }
                // The host derives what the row's move means for the children unless the engine
                // derived it over the record the row moves away from.
                let children_derived =
                    answer.record_damage & FfiStyleInvalidationField::ChildrenDerivedOverOldRecord as u32 != 0;
                if invalidation.recompute_descendants
                    || invalidation.resnaps_scroll_containers()
                    || invalidation.repaints_selection()
                    || (!children_derived && invalidation.reaches_children())
                {
                    return Err(FfiFlightStyleDecline::Descendants);
                }
                answer.record_damage
            };
            if self
                .computed_group_sets
                .style_record_custom_property_environment(answer.old_style_record)
                != self
                    .computed_group_sets
                    .style_record_custom_property_environment(answer.new_style_record)
            {
                return Err(FfiFlightStyleDecline::CustomProperties);
            }
            let holds_resources = |record: u64| {
                let holds_images = self
                    .style_record_dependency_flags(record)
                    .is_none_or(|flags| flags & super::HOLDS_IMAGE_VALUES != 0);
                holds_images
                    || self.computed_group_sets.style_record_view(record).is_none_or(|view| {
                        !ComputedValuesView::new(SharedPayload::as_pointer_slice(view.payloads))
                            .anchor()
                            .anchor_names
                            .as_slice()
                            .is_empty()
                    })
            };
            if holds_resources(answer.old_style_record) || holds_resources(answer.new_style_record) {
                return Err(FfiFlightStyleDecline::Resources);
            }
            if self.style_record_payloads(answer.new_style_record).is_none() {
                return Err(FfiFlightStyleDecline::Row);
            }
            rows.push(FlightStyleRow {
                style_node: node,
                old_style_record: answer.old_style_record,
                new_style_record: answer.new_style_record,
                damage,
                viewport_propagation_source: viewport_propagation_sources.contains(&node),
            });
        }
        Ok(rows)
    }
}
