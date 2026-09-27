/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The reactions an applied style reaction derives for the element's children.
//!
//! What a child reads of its parent is the inherited half of its style, its custom-property
//! environment, and its display; a change confined to anything else reaches no child. C++ reports
//! each reaction it applied together with what its invalidation says moved, and the next
//! transaction's pass turns that into the exact style inputs of the flat-tree children.

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

    /// What applying the reaction to `node` moved, from what the element held before and holds
    /// now, as the host reports it with the application.
    fn style_reaction_row_facts(facts: u32) -> StyleReactionRowFacts {
        if facts & fact::ROW_WAS_UNSTYLED != 0 {
            return StyleReactionRowFacts {
                was_unstyled: true,
                ..Default::default()
            };
        }
        StyleReactionRowFacts {
            was_unstyled: false,
            was_display_none: facts & fact::ROW_WAS_DISPLAY_NONE != 0,
            display_changed: facts & fact::ROW_DISPLAY_CHANGED != 0,
        }
    }

    /// Keep a reaction C++ applied to `node`, for the next transaction's pass to derive the
    /// children's reactions from: `reaction` is what the element reacted to,
    /// `inherited_style_groups_changed` names the inherited groups its style moved, and `facts`
    /// says what else the application found.
    pub fn record_applied_style_reaction(
        &mut self,
        node: StyleNodeID,
        reaction: u8,
        inherited_style_groups_changed: u8,
        facts: u32,
    ) {
        self.host.applied_style_reactions.push(AppliedStyleReaction {
            node,
            reaction,
            inherited_style_groups_changed,
            facts,
        });
    }

    /// Whether a reaction C++ applied is still to derive the children's reactions from.
    #[must_use]
    pub(super) fn has_applied_style_reactions(&self) -> bool {
        !self.host.applied_style_reactions.is_empty()
    }

    /// The children's reactions of every reaction C++ applied since the last transaction, which
    /// join this one. What each element's installed record generates is read from the record.
    pub(super) fn derive_applied_style_reactions(&mut self) {
        let applied = std::mem::take(&mut self.host.applied_style_reactions);
        let mut derived = Vec::new();
        for applied in &applied {
            if !self.retained.tree.is_live(applied.node) {
                continue;
            }
            derived.clear();
            self.derive_applied_style_reaction(applied, &mut derived);
            for child in &derived {
                self.record_derived_element_style_input(child.child, child.reaction, child.groups);
                if child.parent_display_moved {
                    self.retained.parent_inputs_moved_nodes.insert(child.child);
                }
            }
        }
    }

    /// Whether `node` owes a style input: one recorded or derived for it, or one a reaction C++
    /// applied to an element it inherits from derives once the next transaction takes it.
    #[must_use]
    pub fn owes_element_style_input(&self, node: StyleNodeID) -> bool {
        self.has_deferred_element_style_input(node)
            || self.applied_style_reactions_derive_input(node, self.host.applied_style_reactions.iter().copied())
    }

    /// Whether one of the reactions C++ applied, `applied`, derives a style input for `node` once
    /// a transaction takes it: the host holds what it applied since the last transaction until the
    /// next one takes it, and a read of `node` before then owes `node` that input.
    #[must_use]
    pub(super) fn applied_style_reactions_derive_input(
        &self,
        node: StyleNodeID,
        applied: impl IntoIterator<Item = AppliedStyleReaction>,
    ) -> bool {
        let mut derived = Vec::new();
        applied.into_iter().any(|applied| {
            if !self.retained.tree.is_live(applied.node) {
                return false;
            }
            derived.clear();
            self.derive_applied_style_reaction(&applied, &mut derived);
            derived.iter().any(|child| child.child == node && child.reaction != 0)
        })
    }

    fn derive_applied_style_reaction(&self, applied: &AppliedStyleReaction, out: &mut Vec<DerivedChildReaction>) {
        let row_facts = Self::style_reaction_row_facts(applied.facts);
        self.derive_child_reactions(
            applied.node,
            applied.reaction,
            applied.inherited_style_groups_changed,
            applied.facts,
            &row_facts,
            out,
        );
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

        let installed = self.installed_record_state(node);
        // The node leaving display:none, or an ancestor's doing so, reveals its children.
        let reveals_children = ancestor_became_visible
            || (row_facts.was_display_none
                && installed
                    .as_ref()
                    .is_some_and(|installed| !installed.in_display_none_subtree));

        // A slot's assigned elements take their style from the slot, and a slot that moved at all
        // recomputes them. A slot leaving display:none reveals them as it does its children, and
        // they reveal theirs in turn.
        if self.retained.facts.is_slot(node)
            && (!invalidation_is_none || did_change_custom_properties || ancestor_became_visible)
        {
            let reaction = STYLE_REACTION_RECOMPUTE_STYLE
                | if reveals_children {
                    STYLE_REACTION_ANCESTOR_BECAME_VISIBLE
                } else {
                    0
                };
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
        let Some(installed) = installed else {
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
        if reveals_children {
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

/// A reaction C++ applied to an element, as it reported it.
#[derive(Clone, Copy, Debug)]
pub(super) struct AppliedStyleReaction {
    node: StyleNodeID,
    reaction: u8,
    inherited_style_groups_changed: u8,
    facts: u32,
}

impl AppliedStyleReaction {
    /// The reaction the host applied as `reaction`, or `None` for a node that is none.
    pub(super) fn from_host(reaction: &super::bridge::FfiAppliedStyleReaction) -> Option<Self> {
        Some(Self {
            node: StyleNodeID::from_raw(reaction.node)?,
            reaction: reaction.reaction,
            inherited_style_groups_changed: reaction.inherited_style_groups_changed,
            facts: reaction.facts,
        })
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
