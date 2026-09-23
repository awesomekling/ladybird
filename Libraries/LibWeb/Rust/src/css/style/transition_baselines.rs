/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The before-change styles a style stabilization epoch decides CSS transitions against.

use super::RetainedState;
use super::tree::StyleNodeID;

impl RetainedState {
    /// https://drafts.csswg.org/css-transitions-2/#defining-before-change-style
    /// Style, layout or animation feedback can give a target a transition in any later pass of
    /// the epoch, and that transition starts from the style the target held before the epoch's
    /// first pass. The first record named for a target is that style: it is kept, pinned, until
    /// the epoch commits. Says whether this call recorded it.
    pub(crate) fn record_transition_baseline(&mut self, node: StyleNodeID, pseudo_kind: u8, style_record: u64) -> bool {
        if style_record == 0 || self.transition_baselines.contains_key(&(node, pseudo_kind)) {
            return false;
        }
        self.computed_group_sets.pin_style_record(style_record);
        self.transition_baselines.insert((node, pseudo_kind), style_record);
        true
    }

    /// The before-change style the epoch decides the target's transitions against, or 0 before a
    /// pass has recorded one.
    pub(crate) fn transition_baseline(&self, node: StyleNodeID, pseudo_kind: u8) -> u64 {
        self.transition_baselines
            .get(&(node, pseudo_kind))
            .copied()
            .unwrap_or(0)
    }

    /// The epoch committed: no later pass decides against these styles.
    pub(crate) fn release_transition_baselines(&mut self) {
        for (_, style_record) in std::mem::take(&mut self.transition_baselines) {
            self.computed_group_sets.unpin_style_record(style_record);
        }
    }

    /// A retired node's identity can name another element before the epoch commits.
    pub(crate) fn release_transition_baselines_of(&mut self, node: StyleNodeID) {
        let computed_group_sets = &mut self.computed_group_sets;
        self.transition_baselines.retain(|(target, _), style_record| {
            if *target != node {
                return true;
            }
            computed_group_sets.unpin_style_record(*style_record);
            false
        });
    }
}
