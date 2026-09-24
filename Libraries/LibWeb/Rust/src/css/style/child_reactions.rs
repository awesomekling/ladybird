/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The reactions an applied style reaction derives for the element's children.
//!
//! What a child reads of its parent is the inherited half of its style, its custom-property
//! environment, and its display; a change confined to anything else reaches no child. C++ reports
//! each reaction it applied together with what its invalidation says moved, and the engine turns
//! that into the exact style inputs of the flat-tree children for the next transaction.

use super::bridge::style_reaction_applied_fact as fact;
use super::transaction::{
    STYLE_REACTION_ANCESTOR_BECAME_VISIBLE, STYLE_REACTION_INHERITED_CUSTOM_PROPERTIES, STYLE_REACTION_INHERITED_STYLE,
    STYLE_REACTION_RECOMPUTE_DESCENDANT_STYLES, STYLE_REACTION_RECOMPUTE_STYLE,
};
use super::{StyleEngineState, StyleNodeID};
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::host_shared::SharedPayload;

/// Every inherited style group, for a change that reaches all of them.
const ALL_INHERITED_STYLE_GROUPS: u8 = (1 << 7) - 1;

/// What applying a style reaction moved of the element's own state, as its children read it.
#[derive(Default)]
struct StyleReactionRowFacts {
    was_unstyled: bool,
    was_display_none: bool,
    /// The element's computed display moved, which its children's box-type transformation reads.
    display_changed: bool,
}

/// What an element's installed record generates, as its children read it.
struct InstalledRecordState {
    is_display_none: bool,
    in_display_none_subtree: bool,
}

impl StyleEngineState {
    /// What the element's installed record generates, or `None` for an element without style.
    fn installed_record_state(&self, node: StyleNodeID) -> Option<InstalledRecordState> {
        let record = self.retained.computed_group_sets.assigned_style_record(node)?;
        let view = self
            .retained
            .computed_group_sets
            .style_record_view(record.raw())
            .expect("installed style record is live");
        let values = ComputedValuesView::new(SharedPayload::as_pointer_slice(view.payloads));
        Some(InstalledRecordState {
            is_display_none: values.display().is_none(),
            in_display_none_subtree: view.dependency_flags & (1 << 2) != 0,
        })
    }

    /// The host begins applying a style reaction to `node`: what the element holds now is what
    /// the application moves it from.
    pub fn begin_style_reaction(&mut self, node: StyleNodeID) {
        self.host.style_reaction_row_start = Some((node, self.host.held_style_record_displays.get(&node).copied()));
    }

    /// What applying the reaction that began on `node` moved, from what the element held before
    /// and holds now.
    fn style_reaction_row_facts(&mut self, node: StyleNodeID) -> StyleReactionRowFacts {
        // The host begins every reaction it applies on the element it applies it to. Should it not,
        // nothing says what the element held before, and its children are told it was unstyled:
        // that owes them everything.
        let start = self.host.style_reaction_row_start.take();
        // A row whose children the engine derived is applied without beginning on it: the engine
        // derived them over the record it names, and the host applied it over that.
        let derived_over = self
            .retained
            .engine_row_child_facts
            .get(&node)
            .map(|row| row.old_style_record);
        let before = match (start, derived_over) {
            (Some((row_node, before)), _) if row_node == node => before,
            (_, Some(old_style_record)) => (old_style_record != 0).then(|| self.record_display(old_style_record)),
            _ => {
                debug_assert!(false, "style reaction applied to {node:?} without beginning on it");
                super::seal::note_broken_assumption("StyleReactionAppliedWithoutBeginning");
                None
            }
        };
        let now = self.host.held_style_record_displays.get(&node).copied();
        // An element left without style by the application had none before it either.
        debug_assert!(
            now.is_some() || before.is_none(),
            "style reaction cleared the style of {node:?}"
        );
        let Some(before) = before else {
            return StyleReactionRowFacts {
                was_unstyled: true,
                ..Default::default()
            };
        };
        StyleReactionRowFacts {
            was_unstyled: false,
            was_display_none: before.is_some_and(|display| display.is_none()),
            display_changed: matches!((before, now), (Some(before), Some(Some(now))) if before != now),
        }
    }

    /// Derive the children's reactions from a reaction C++ applied to `node`: `reaction` is what
    /// the element reacted to, `inherited_style_groups_changed` names the inherited groups its
    /// style moved, and `facts` says what else the application found. What the element's
    /// installed record generates is read from the record.
    pub fn note_style_reaction_applied(
        &mut self,
        node: StyleNodeID,
        reaction: u8,
        inherited_style_groups_changed: u8,
        facts: u32,
    ) {
        let row_facts = self.style_reaction_row_facts(node);
        let mut derived = Vec::new();
        self.derive_child_reactions(
            node,
            reaction,
            inherited_style_groups_changed,
            facts,
            &row_facts,
            &mut derived,
        );
        for child in derived {
            self.record_derived_element_style_input(child.child, child.reaction, child.groups);
            if child.parent_display_moved {
                self.retained.parent_inputs_moved_nodes.insert(child.child);
            }
        }
    }

    /// The display a record generates, as the host's held-record mirror keeps it.
    fn record_display(&self, style_record: u64) -> Option<crate::css::display::FfiDisplay> {
        self.retained
            .computed_group_sets
            .style_record_payloads(style_record)
            .filter(|payloads| payloads.len() > crate::css::computed_value_types::STYLE_GROUP_INDEX_BOX)
            .map(|payloads| {
                ComputedValuesView::new(SharedPayload::as_pointer_slice(payloads))
                    .box_values()
                    .display
            })
    }

    /// What the engine says a settled row moved, for the children: the facts the host reports
    /// with its application, derived from the two records and their damage instead.
    fn engine_row_child_facts(&self, node: StyleNodeID, row: &EngineRowChildFacts) -> (u8, u32, StyleReactionRowFacts) {
        let mut facts = 0;
        // The host installs only the pseudo-elements it generates boxes for: ::backdrop only in
        // the top layer.
        let in_top_layer = self.retained.computed_group_sets.adjustment_facts(node)
            & super::bridge::element_adjustment_fact::RENDERED_IN_TOP_LAYER
            != 0;
        let pseudo_damage = row
            .pseudo_damages
            .iter()
            .filter(|&&(kind, _)| matches!(kind, 0 | 2 | 3 | 5 | 6) || (kind == 1 && in_top_layer))
            .fold(0, |damage, &(_, pseudo_damage)| damage | pseudo_damage);
        let (groups, is_none, rebuild, recompute_descendants) = if row.old_style_record == 0 {
            // A first style is a full invalidation.
            let pseudo = super::style_invalidation::unpack_invalidation(pseudo_damage);
            (pseudo.inherited_groups, false, true, pseudo.recompute_descendants)
        } else {
            let element = if row.old_style_record == row.new_style_record {
                0
            } else {
                row.element_damage
            };
            let mut invalidation = super::style_invalidation::unpack_invalidation(element);
            invalidation.merge_packed(pseudo_damage);
            if row.root_font_metrics_moved {
                invalidation.recompute_descendants = true;
            }
            (
                invalidation.inherited_groups,
                invalidation.requires_nothing(),
                invalidation.level >= 3,
                invalidation.recompute_descendants,
            )
        };
        if is_none {
            facts |= fact::INVALIDATION_IS_NONE;
        }
        if rebuild {
            facts |= fact::NEEDS_LAYOUT_TREE_REBUILD;
        }
        if recompute_descendants {
            facts |= fact::RECOMPUTE_DESCENDANT_STYLES;
        }
        if self.retained.children_explicitly_inherit_marks.contains(&node) {
            facts |= fact::CHILDREN_EXPLICITLY_INHERIT;
        }
        if self
            .retained
            .tree
            .shadow_root_of(node)
            .is_some_and(|root| self.retained.children_explicitly_inherit_marks.contains(&root))
        {
            facts |= fact::SHADOW_CHILDREN_EXPLICITLY_INHERIT;
        }
        let row_facts = if row.old_style_record == 0 {
            StyleReactionRowFacts {
                was_unstyled: true,
                ..Default::default()
            }
        } else {
            let before = self.record_display(row.old_style_record);
            let now = self.record_display(row.new_style_record);
            StyleReactionRowFacts {
                was_unstyled: false,
                was_display_none: before.is_some_and(|display| display.is_none()),
                display_changed: matches!((before, now), (Some(before), Some(now)) if before != now),
            }
        };
        (groups, facts, row_facts)
    }

    /// The reactions applying the row the engine settled for `node` derives for its children, as
    /// (child, reaction, inherited style groups, parent display moved).
    pub(super) fn derive_engine_row_child_reactions(
        &self,
        node: StyleNodeID,
        reaction: u8,
        row: &EngineRowChildFacts,
        out: &mut Vec<(StyleNodeID, u8, u8, bool)>,
    ) {
        let (groups, facts, row_facts) = self.engine_row_child_facts(node, row);
        let mut derived = Vec::new();
        self.derive_child_reactions(node, reaction, groups, facts, &row_facts, &mut derived);
        out.extend(
            derived
                .into_iter()
                .map(|child| (child.child, child.reaction, child.groups, child.parent_display_moved)),
        );
    }

    /// The reactions a reaction applied to `node` derives for its children.
    fn derive_child_reactions(
        &self,
        node: StyleNodeID,
        reaction: u8,
        inherited_style_groups_changed: u8,
        facts: u32,
        row_facts: &StyleReactionRowFacts,
        out: &mut Vec<DerivedChildReaction>,
    ) {
        let has = |bit: u32| facts & bit != 0;
        let did_change_custom_properties = has(fact::DID_CHANGE_CUSTOM_PROPERTIES);
        let invalidation_is_none = has(fact::INVALIDATION_IS_NONE);
        let ancestor_became_visible = reaction & STYLE_REACTION_ANCESTOR_BECAME_VISIBLE != 0;

        // A slot's assigned elements take their style from the slot, and a slot that moved at all
        // recomputes them. A slot leaving display:none reveals them as it does its children.
        if self.retained.facts.is_slot(node)
            && (!invalidation_is_none || did_change_custom_properties || ancestor_became_visible)
        {
            let reaction = STYLE_REACTION_RECOMPUTE_STYLE | (reaction & STYLE_REACTION_ANCESTOR_BECAME_VISIBLE);
            for index in 0..self.retained.tree.assigned_nodes_of(node).len() {
                let assigned = self.retained.tree.assigned_nodes_of(node)[index];
                // A text slottable holds a place in the list but has no style of its own to recompute.
                if assigned.is_text() {
                    continue;
                }
                out.push(DerivedChildReaction {
                    child: assigned,
                    reaction,
                    groups: 0,
                    parent_display_moved: false,
                });
            }
        }

        // A descendant whose style was cleared on entry to display:none stays unmaterialized.
        let Some(installed) = self.installed_record_state(node) else {
            return;
        };

        if installed.is_display_none {
            let (child_reaction, groups) = if row_facts.was_unstyled {
                (STYLE_REACTION_RECOMPUTE_STYLE, 0)
            } else {
                let mut child_reaction = 0;
                if did_change_custom_properties {
                    child_reaction |= STYLE_REACTION_INHERITED_CUSTOM_PROPERTIES;
                }
                if inherited_style_groups_changed != 0 {
                    child_reaction |= STYLE_REACTION_INHERITED_STYLE;
                }
                if has(fact::RECOMPUTE_DESCENDANT_STYLES) {
                    child_reaction |= STYLE_REACTION_RECOMPUTE_DESCENDANT_STYLES;
                }
                (child_reaction, inherited_style_groups_changed)
            };
            if child_reaction == 0 {
                return;
            }
            for parent in [Some(node), self.retained.tree.shadow_root_of(node)]
                .into_iter()
                .flatten()
            {
                let mut next = self.retained.tree.first_element_child(parent);
                while let Some(child) = next {
                    next = self.retained.tree.next_element_sibling(child);
                    if parent != node || self.retained.tree.assigned_slot_of(child).is_none() {
                        out.push(DerivedChildReaction {
                            child,
                            reaction: child_reaction,
                            groups,
                            parent_display_moved: false,
                        });
                    }
                }
            }
            return;
        }

        let mut common_child_reaction = 0;
        if reaction & STYLE_REACTION_INHERITED_CUSTOM_PROPERTIES != 0 || did_change_custom_properties {
            common_child_reaction |= STYLE_REACTION_INHERITED_CUSTOM_PROPERTIES;
        }
        if has(fact::NEEDS_LAYOUT_TREE_REBUILD) {
            common_child_reaction |= STYLE_REACTION_RECOMPUTE_STYLE;
        }
        if reaction & STYLE_REACTION_RECOMPUTE_DESCENDANT_STYLES != 0 || has(fact::RECOMPUTE_DESCENDANT_STYLES) {
            common_child_reaction |= STYLE_REACTION_RECOMPUTE_DESCENDANT_STYLES;
        }
        if ancestor_became_visible || (row_facts.was_display_none && !installed.in_display_none_subtree) {
            common_child_reaction |= STYLE_REACTION_ANCESTOR_BECAME_VISIBLE;
        }

        let child_reaction = |children_explicitly_inherit: bool| {
            let groups = if !invalidation_is_none && children_explicitly_inherit {
                ALL_INHERITED_STYLE_GROUPS
            } else {
                inherited_style_groups_changed
            };
            let reaction = common_child_reaction | if groups != 0 { STYLE_REACTION_INHERITED_STYLE } else { 0 };
            (reaction, groups)
        };
        // A child's box-type transformation reads its parent's display: when that moved, the
        // child's record is driven again in full, whatever its own winners did.
        let display_changed = row_facts.display_changed;
        let mut next = self.retained.tree.first_element_child(node);
        while let Some(child) = next {
            next = self.retained.tree.next_element_sibling(child);
            if self.retained.tree.assigned_slot_of(child).is_some() {
                continue;
            }
            let (light_reaction, light_groups) = child_reaction(
                !invalidation_is_none
                    && (has(fact::CHILDREN_EXPLICITLY_INHERIT)
                        || self.node_explicitly_inherits_non_inherited_property(child)),
            );
            out.push(DerivedChildReaction {
                child,
                reaction: light_reaction,
                groups: light_groups,
                parent_display_moved: display_changed,
            });
        }
        let mut next = self
            .tree
            .shadow_root_of(node)
            .and_then(|root| self.retained.tree.first_element_child(root));
        while let Some(child) = next {
            next = self.retained.tree.next_element_sibling(child);
            let (shadow_reaction, shadow_groups) = child_reaction(
                !invalidation_is_none
                    && (has(fact::SHADOW_CHILDREN_EXPLICITLY_INHERIT)
                        || self.node_explicitly_inherits_non_inherited_property(child)),
            );
            out.push(DerivedChildReaction {
                child,
                reaction: shadow_reaction,
                groups: shadow_groups,
                parent_display_moved: display_changed,
            });
        }
    }
}

/// One child reaction a parent's applied reaction derives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DerivedChildReaction {
    child: StyleNodeID,
    reaction: u8,
    groups: u8,
    /// The parent's display moved, which the child's box-type transformation reads.
    parent_display_moved: bool,
}

/// What the engine settled for a row, kept until the host applies it.
#[derive(Clone)]
pub(super) struct EngineRowChildFacts {
    pub(super) old_style_record: u64,
    pub(super) new_style_record: u64,
    pub(super) element_damage: u32,
    pub(super) pseudo_damages: Vec<(u8, u32)>,
    pub(super) root_font_metrics_moved: bool,
}
